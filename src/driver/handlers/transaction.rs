use crate::driver::handlers::DriverHandler;
use crate::driver::protocol::{Command, DriverError, IsolationLevel, Response};
use crate::transaction::IsolationLevel as TxIsolationLevel;

pub fn handle_begin_transaction(
    handler: &mut DriverHandler,
    database: String,
    isolation_level: IsolationLevel,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(_) => {
            let tx_isolation: TxIsolationLevel = isolation_level.into();
            match handler.storage.transaction_manager() {
                Ok(tx_manager) => match tx_manager.begin(tx_isolation) {
                    Ok(tx_id) => {
                        let tx_id_str = tx_id.to_string();
                        handler.transactions.insert(tx_id_str.clone(), tx_id);
                        Response::ok_tx(tx_id_str)
                    }
                    Err(e) => Response::error(DriverError::TransactionError(e.to_string())),
                },
                Err(e) => Response::error(DriverError::TransactionError(e.to_string())),
            }
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_commit_transaction(handler: &mut DriverHandler, tx_id: String) -> Response {
    match handler.transactions.remove(&tx_id) {
        Some(tx) => {
            // Read the touched collections before commit consumes the
            // transaction; the cache is dropped only once the writes landed.
            let touched = touched_collections(handler, tx);
            let storage = &handler.storage;
            match off_the_worker(|| storage.commit_transaction(tx)) {
                Ok(_) => {
                    for c in touched {
                        crate::storage::query_cache::get_query_cache().invalidate_collection(&c);
                    }
                    Response::ok_empty()
                }
                Err(e) => Response::error(DriverError::TransactionError(e.to_string())),
            }
        }
        None => Response::error(DriverError::TransactionError(
            "Transaction not found".to_string(),
        )),
    }
}

/// Begin, run each step in the transaction, commit: a whole transaction in
/// one round trip (`Command::RunTransaction`).
///
/// Answers `{"committed": true, "results": [...]}`, one result per step as
/// `transaction_command` would have answered it, or `{"committed": false,
/// "stopped_at": i}` when step `i` has `require_rows` and its query returned
/// no row: the transaction is rolled back, nothing it staged lands. A step
/// that fails rolls it back too, and its error names the step.
pub async fn handle_run_transaction(
    handler: &mut DriverHandler,
    database: String,
    isolation_level: IsolationLevel,
    steps: Vec<crate::driver::protocol::TransactionStep>,
) -> Response {
    for (i, step) in steps.iter().enumerate() {
        if !matches!(
            step.command,
            Command::Insert { .. }
                | Command::Update { .. }
                | Command::Delete { .. }
                | Command::Query { .. }
        ) {
            return Response::error(DriverError::TransactionError(format!(
                "step {i}: only insert, update, delete and query can run in a transaction"
            )));
        }
        if step.command.database() != Some(database.as_str()) {
            return Response::error(DriverError::TransactionError(format!(
                "step {i}: every step must name the transaction's database, {database}"
            )));
        }
    }
    let tx_id = match handle_begin_transaction(handler, database.clone(), isolation_level) {
        Response::Ok {
            tx_id: Some(id), ..
        } => id,
        other => return other,
    };
    let mut results = Vec::with_capacity(steps.len());
    for (i, step) in steps.into_iter().enumerate() {
        // Each step needs what it would need on its own. A query that
        // writes is upgraded to Write when it is staged.
        if let Some(action) = step.command.required_action() {
            if let Err(e) = crate::server::AuthorizationService::check_permission_raw(
                &handler.session_permissions,
                action,
                Some(&database),
                handler.session_scoped_databases.as_deref(),
            ) {
                let _ = handle_rollback_transaction(handler, tx_id);
                return Response::error(DriverError::AuthError(format!("step {i}: {e}")));
            }
        }
        let response =
            handle_transaction_command(handler, tx_id.clone(), Box::new(step.command)).await;
        let value = match step_value(response) {
            Ok(value) => value,
            Err(e) => {
                let _ = handle_rollback_transaction(handler, tx_id);
                return Response::error(at_step(i, e));
            }
        };
        if step.require_rows && value.as_array().is_none_or(|rows| rows.is_empty()) {
            let _ = handle_rollback_transaction(handler, tx_id);
            return Response::ok(serde_json::json!({"committed": false, "stopped_at": i}));
        }
        results.push(value);
    }
    match handle_commit_transaction(handler, tx_id) {
        Response::Ok { .. } => {
            Response::ok(serde_json::json!({"committed": true, "results": results}))
        }
        other => other,
    }
}

/// A step's answer as a value: rows a read-only query sent already encoded
/// (shared with the cache, or copied from storage) are decoded.
fn step_value(response: Response) -> Result<serde_json::Value, DriverError> {
    match response {
        Response::Ok { data, .. } => Ok(data.unwrap_or(serde_json::Value::Null)),
        Response::Rows { data } => Ok(serde_json::Value::Array(data.0.as_ref().clone())),
        Response::RawRows { data } => rmp_serde::from_slice(&data.0)
            .map_err(|e| DriverError::ProtocolError(format!("rows: {e}"))),
        Response::Error { error } => Err(error),
        Response::Pong { .. } | Response::Batch { .. } => Err(DriverError::ProtocolError(
            "unexpected answer to a transaction step".to_string(),
        )),
    }
}

/// `e`, saying which step it came from.
fn at_step(i: usize, e: DriverError) -> DriverError {
    match e {
        DriverError::ConnectionError(m) => DriverError::ConnectionError(format!("step {i}: {m}")),
        DriverError::ProtocolError(m) => DriverError::ProtocolError(format!("step {i}: {m}")),
        DriverError::DatabaseError(m) => DriverError::DatabaseError(format!("step {i}: {m}")),
        DriverError::AuthError(m) => DriverError::AuthError(format!("step {i}: {m}")),
        DriverError::TransactionError(m) => DriverError::TransactionError(format!("step {i}: {m}")),
        DriverError::InvalidCommand(m) => DriverError::InvalidCommand(format!("step {i}: {m}")),
        other => other,
    }
}

/// Run work that waits (a commit waits for its fsync) without holding up
/// the other connections this async worker serves: `block_in_place` hands
/// the worker's queue to another thread while it runs, and costs nothing
/// when that queue is empty (one client). The server runs on the
/// multi-threaded runtime; anywhere else (unit tests) the work runs as is.
fn off_the_worker<T>(work: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()) {
        Ok(tokio::runtime::RuntimeFlavor::MultiThread) => tokio::task::block_in_place(work),
        _ => work(),
    }
}

/// Names of the collections a transaction has written to so far.
fn touched_collections(
    handler: &DriverHandler,
    id: crate::transaction::TransactionId,
) -> Vec<String> {
    let Ok(manager) = handler.storage.transaction_manager() else {
        return Vec::new();
    };
    let Ok(tx) = manager.get(id) else {
        return Vec::new();
    };
    let tx = tx.read().unwrap();
    let mut names: Vec<String> = tx
        .operations
        .iter()
        .map(|op| op.collection().to_string())
        .collect();
    names.sort();
    names.dedup();
    names
}

pub fn handle_rollback_transaction(handler: &mut DriverHandler, tx_id: String) -> Response {
    match handler.transactions.remove(&tx_id) {
        Some(tx) => match handler.storage.rollback_transaction(tx) {
            Ok(_) => Response::ok_empty(),
            Err(e) => Response::error(DriverError::TransactionError(e.to_string())),
        },
        None => Response::error(DriverError::TransactionError(
            "Transaction not found".to_string(),
        )),
    }
}

/// Run one command inside an open transaction.
///
/// Single-document writes (`Insert`, `Update`, `Delete`) and queries that
/// write are staged: recorded on the transaction and applied at commit, so a
/// rollback discards them. A read-only query reads committed data. Anything
/// else is refused. It used to run the inner command outside the
/// transaction, which committed the write immediately and made `Rollback` a
/// no-op without saying so.
pub async fn handle_transaction_command(
    handler: &mut DriverHandler,
    tx_id: String,
    command: Box<Command>,
) -> Response {
    let Some(&id) = handler.transactions.get(&tx_id) else {
        return Response::error(DriverError::TransactionError(
            "Transaction not found".to_string(),
        ));
    };

    match *command {
        Command::Insert {
            database,
            collection,
            key,
            mut document,
        } => {
            if let Some(k) = key {
                if let Some(obj) = document.as_object_mut() {
                    obj.insert("_key".to_string(), serde_json::json!(k));
                }
            }
            stage(
                handler,
                id,
                &database,
                &collection,
                |coll, tx, wal, locks| {
                    coll.insert_tx(tx, wal, locks, document)
                        .map(|doc| Some(doc.to_value()))
                },
            )
        }
        // `update_tx` always merges into the stored document, like `update`.
        Command::Update {
            database,
            collection,
            key,
            document,
            ..
        } => stage(
            handler,
            id,
            &database,
            &collection,
            |coll, tx, wal, locks| {
                coll.update_tx(tx, wal, locks, &key, document)
                    .map(|doc| Some(doc.to_value()))
            },
        ),
        Command::Delete {
            database,
            collection,
            key,
        } => stage(
            handler,
            id,
            &database,
            &collection,
            |coll, tx, wal, locks| coll.delete_tx(tx, wal, locks, &key).map(|_| None),
        ),
        // A query runs as it does through HTTP's `…/transaction/{tx}/query`:
        // it reads committed data, so it does not see this transaction's
        // staged writes, and the writes of one that mutates are staged on
        // the transaction instead of applied.
        Command::Query {
            database,
            sdbql,
            bind_vars,
            cache,
        } => {
            let prepared =
                match crate::sdbql::get_prepared_statement_cache().parse_if_needed(&sdbql) {
                    Ok(p) => p,
                    Err(e) => {
                        return Response::error(DriverError::DatabaseError(format!(
                            "Parse error: {}",
                            e
                        )))
                    }
                };
            if !prepared.query.has_mutations() {
                return Box::pin(handler.execute_command(Command::Query {
                    database,
                    sdbql,
                    bind_vars,
                    cache,
                }))
                .await;
            }
            stage_query(
                handler,
                id,
                database,
                prepared.query.clone(),
                bind_vars.unwrap_or_default(),
            )
            .await
        }
        _ => Response::error(DriverError::TransactionError(
            "only Insert, Update, Delete and Query can run inside a transaction".to_string(),
        )),
    }
}

/// Stage a query that writes on transaction `id`, as the HTTP transactional
/// query does: the executor records its writes on the transaction, they land
/// at commit, and an `UPDATE` computed from a document the query read is
/// refused with a conflict if that document changed since (card #796). The
/// rows it returns come back as for any query, so a client tells whether a
/// filtered write matched from them (`… UPDATE a … RETURN 1`).
async fn stage_query(
    handler: &DriverHandler,
    id: crate::transaction::TransactionId,
    database: String,
    query: std::sync::Arc<crate::sdbql::Query>,
    bind_vars: std::collections::HashMap<String, serde_json::Value>,
) -> Response {
    // The dispatch check only required Read for a query.
    if let Err(e) = crate::server::AuthorizationService::check_permission_raw(
        &handler.session_permissions,
        crate::server::PermissionAction::Write,
        Some(&database),
        handler.session_scoped_databases.as_deref(),
    ) {
        return Response::error(DriverError::AuthError(e.to_string()));
    }
    if query.has_unstageable_writes() {
        return Response::error(DriverError::TransactionError(
            "streams, materialized views and state-changing functions cannot run inside a \
             transaction; they would apply immediately and survive a rollback"
                .to_string(),
        ));
    }
    let manager = match handler.storage.transaction_manager() {
        Ok(m) => m,
        Err(e) => return Response::error(DriverError::TransactionError(e.to_string())),
    };
    let writer = match manager.get(id) {
        Ok(tx) => crate::sdbql::executor::TxWriter {
            tx,
            wal: manager.wal().clone(),
            locks: manager.lock_manager().clone(),
        },
        Err(e) => return Response::error(DriverError::TransactionError(e.to_string())),
    };
    let principal = handler.query_principal(&database);
    let storage = handler.storage.clone();
    let inline = is_point_statement(&query);
    let run = move || {
        crate::sdbql::QueryExecutor::with_database_and_bind_vars(&storage, database, bind_vars)
            .with_principal(principal)
            .with_timeout(std::time::Duration::from_secs(
                super::query::QUERY_TIMEOUT_SECS,
            ))
            .with_transaction(writer)
            .execute(&query)
    };
    let outcome = if inline {
        run()
    } else {
        // Anything that may loop or scan: on the blocking pool, as for any
        // query.
        match tokio::task::spawn_blocking(run).await {
            Ok(outcome) => outcome,
            Err(e) => {
                return Response::error(DriverError::DatabaseError(format!(
                    "Task join error: {}",
                    e
                )))
            }
        }
    };
    match outcome {
        Ok(rows) => Response::ok(serde_json::Value::Array(rows)),
        Err(e) => Response::error(DriverError::TransactionError(e.to_string())),
    }
}

/// Whether a staged statement reaches documents only by key: `FOR d IN c
/// FILTER d._key == <constant or bind variable> [FILTER …] [LET …]` then one
/// `UPDATE` / `REPLACE` / `REMOVE`, or a single `INSERT`, with no heavy
/// expression anywhere. Staging one is a few microseconds of work, less
/// than the round trip to the blocking pool (two thread wake-ups), so it
/// runs inline, as point reads do in `handle_query`.
fn is_point_statement(query: &crate::sdbql::Query) -> bool {
    use crate::sdbql::ast::BodyClause;
    use crate::server::handlers::query::expression_is_heavy as heavy;
    if !query.set_operations.is_empty()
        || !query.join_clauses.is_empty()
        || query.with_clause.is_some()
        || query.window_clause.is_some()
        || query.let_clauses.iter().any(|l| heavy(&l.expression))
        || query
            .return_clause
            .as_ref()
            .is_some_and(|r| heavy(&r.expression))
    {
        return false;
    }
    match query.body_clauses.as_slice() {
        [BodyClause::Insert(insert)] => !heavy(&insert.document),
        [BodyClause::For(f), rest @ ..] => {
            if f.source_expression.is_some()
                || f.system_time.is_some()
                || f.valid_time.is_some()
                || f.source_variable
                    .as_ref()
                    .is_some_and(|s| s != &f.collection)
            {
                return false;
            }
            let Some((write, between)) = rest.split_last() else {
                return false;
            };
            let write_light = match write {
                BodyClause::Update(u) => !heavy(&u.selector) && !heavy(&u.changes),
                BodyClause::Remove(r) => !heavy(&r.selector),
                _ => false,
            };
            let mut keyed = false;
            let between_light = between.iter().all(|clause| match clause {
                BodyClause::Filter(filter) => {
                    keyed |= key_equality(&filter.expression, &f.variable);
                    !heavy(&filter.expression)
                }
                BodyClause::Let(l) => !heavy(&l.expression),
                _ => false,
            });
            write_light && between_light && keyed
        }
        _ => false,
    }
}

/// `var._key == <constant or bind variable>`, either way round, possibly
/// one side of an `AND`.
fn key_equality(e: &crate::sdbql::ast::Expression, var: &str) -> bool {
    use crate::sdbql::ast::{BinaryOperator, Expression};
    let is_key = |e: &Expression| {
        matches!(e, Expression::FieldAccess(base, field)
            if field == "_key" && matches!(base.as_ref(), Expression::Variable(v) if v == var))
    };
    let constant =
        |e: &Expression| matches!(e, Expression::BindVariable(_) | Expression::Literal(_));
    match e {
        Expression::BinaryOp {
            left,
            op: BinaryOperator::Equal,
            right,
        } => (is_key(left) && constant(right)) || (is_key(right) && constant(left)),
        Expression::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => key_equality(left, var) || key_equality(right, var),
        _ => false,
    }
}

/// Resolve the collection through the write getter (so the protection tiers
/// apply, as for a plain driver write) and hand it the transaction.
fn stage<F>(
    handler: &DriverHandler,
    id: crate::transaction::TransactionId,
    database: &str,
    collection: &str,
    op: F,
) -> Response
where
    F: FnOnce(
        &crate::storage::Collection,
        &mut crate::transaction::Transaction,
        &std::sync::Arc<crate::transaction::wal::WalWriter>,
        &std::sync::Arc<crate::transaction::lock_manager::LockManager>,
    ) -> crate::error::DbResult<Option<serde_json::Value>>,
{
    let coll = match handler.get_collection_for_write(database, collection) {
        Ok(c) => c,
        Err(e) => return Response::error(e),
    };
    let manager = match handler.storage.transaction_manager() {
        Ok(m) => m,
        Err(e) => return Response::error(DriverError::TransactionError(e.to_string())),
    };
    let tx_arc = match manager.get(id) {
        Ok(t) => t,
        Err(e) => return Response::error(DriverError::TransactionError(e.to_string())),
    };
    let mut tx = tx_arc.write().unwrap();
    match op(&coll, &mut tx, manager.wal(), manager.lock_manager()) {
        Ok(Some(value)) => Response::ok(value),
        Ok(None) => Response::ok_empty(),
        Err(e) => Response::error(DriverError::TransactionError(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::protocol::IsolationLevel;
    use crate::storage::StorageEngine;
    use serde_json::json;
    use std::sync::Arc;

    fn handler() -> (tempfile::TempDir, DriverHandler) {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
        engine.create_database("d".to_string()).unwrap();
        engine
            .get_database("d")
            .unwrap()
            .create_collection("c".to_string(), None)
            .unwrap();
        (tmp, DriverHandler::new(engine, None))
    }

    fn begin(h: &mut DriverHandler) -> String {
        match handle_begin_transaction(h, "d".to_string(), IsolationLevel::ReadCommitted) {
            Response::Ok {
                tx_id: Some(id), ..
            } => id,
            other => panic!("begin failed: {:?}", other),
        }
    }

    fn insert(key: &str) -> Box<Command> {
        Box::new(Command::Insert {
            database: "d".to_string(),
            collection: "c".to_string(),
            key: Some(key.to_string()),
            document: json!({"v": 1}),
        })
    }

    fn exists(h: &DriverHandler, key: &str) -> bool {
        h.get_collection("d", "c").unwrap().get(key).is_ok()
    }

    #[tokio::test]
    async fn rollback_discards_staged_insert() {
        let (_t, mut h) = handler();
        let tx = begin(&mut h);
        let r = handle_transaction_command(&mut h, tx.clone(), insert("k1")).await;
        assert!(matches!(r, Response::Ok { .. }), "{:?}", r);
        assert!(!exists(&h, "k1"), "staged write must not be visible yet");
        handle_rollback_transaction(&mut h, tx);
        assert!(!exists(&h, "k1"));
    }

    #[tokio::test]
    async fn commit_applies_staged_insert() {
        let (_t, mut h) = handler();
        let tx = begin(&mut h);
        handle_transaction_command(&mut h, tx.clone(), insert("k2")).await;
        let r = handle_commit_transaction(&mut h, tx);
        assert!(matches!(r, Response::Ok { .. }), "{:?}", r);
        assert!(exists(&h, "k2"));
    }

    #[tokio::test]
    async fn committed_transaction_records_version_history() {
        let (_t, mut h) = handler();
        h.get_collection("d", "c")
            .unwrap()
            .enable_versioning()
            .unwrap();
        let tx = begin(&mut h);
        handle_transaction_command(&mut h, tx.clone(), insert("k3")).await;
        let upd = Box::new(Command::Update {
            database: "d".to_string(),
            collection: "c".to_string(),
            key: "k3".to_string(),
            document: json!({"v": 2}),
            merge: true,
        });
        handle_transaction_command(&mut h, tx.clone(), upd).await;
        handle_commit_transaction(&mut h, tx);

        let coll = h.get_collection("d", "c").unwrap();
        let history = coll.doc_history("k3");
        assert_eq!(history.len(), 2, "{:?}", history);
        assert_eq!(history[0]["value"]["v"], json!(2));
        assert_eq!(history[1]["value"]["v"], json!(1));
    }

    fn query(sdbql: &str, binds: serde_json::Value) -> Box<Command> {
        Box::new(Command::Query {
            database: "d".to_string(),
            sdbql: sdbql.to_string(),
            bind_vars: serde_json::from_value(binds).unwrap(),
            cache: false,
        })
    }

    /// `Query` is dispatched through `execute_command`, which insists on a
    /// session.
    fn admin_session(h: &mut DriverHandler) {
        h.authenticated_db = Some("d".to_string());
        h.session_permissions
            .insert(crate::server::Permission::global_admin());
    }

    fn balance(h: &DriverHandler, key: &str) -> serde_json::Value {
        h.get_collection("d", "c")
            .unwrap()
            .get(key)
            .unwrap()
            .to_value()["n"]
            .clone()
    }

    #[tokio::test]
    async fn query_in_transaction_reads_committed_data() {
        let (_t, mut h) = handler();
        admin_session(&mut h);
        let tx = begin(&mut h);
        let r =
            handle_transaction_command(&mut h, tx.clone(), query("FOR x IN c RETURN x", json!({})))
                .await;
        // Either `Ok` or the raw-rows fast path; what matters is it ran.
        assert!(!matches!(r, Response::Error { .. }), "{:?}", r);
        handle_commit_transaction(&mut h, tx);
    }

    /// A query that writes is staged: invisible until the commit, applied by
    /// it, and its rows say whether a filtered write matched.
    #[tokio::test]
    async fn a_query_that_writes_is_staged_until_the_commit() {
        let (_t, mut h) = handler();
        admin_session(&mut h);
        h.get_collection("d", "c")
            .unwrap()
            .insert(json!({"_key": "a", "n": 10}))
            .unwrap();
        let debit = "FOR x IN c FILTER x._key == @k FILTER x.n >= @v \
                     UPDATE x WITH {n: x.n - @v} IN c RETURN 1";
        let tx = begin(&mut h);
        let r =
            handle_transaction_command(&mut h, tx.clone(), query(debit, json!({"k": "a", "v": 4})))
                .await;
        assert!(
            matches!(&r, Response::Ok { data: Some(d), .. } if d == &json!([1])),
            "{:?}",
            r
        );
        // Too much: the FILTER drops the row, nothing is staged.
        let r = handle_transaction_command(
            &mut h,
            tx.clone(),
            query(debit, json!({"k": "a", "v": 99})),
        )
        .await;
        assert!(
            matches!(&r, Response::Ok { data: Some(d), .. } if d == &json!([])),
            "{:?}",
            r
        );
        let r = handle_transaction_command(
            &mut h,
            tx.clone(),
            query("INSERT {_key: 'ledger1', v: 4} INTO c", json!({})),
        )
        .await;
        assert!(!matches!(r, Response::Error { .. }), "{:?}", r);
        assert_eq!(balance(&h, "a"), json!(10), "staged, not applied");
        assert!(!exists(&h, "ledger1"));

        assert!(matches!(
            handle_commit_transaction(&mut h, tx),
            Response::Ok { .. }
        ));
        assert_eq!(balance(&h, "a"), json!(6));
        assert!(exists(&h, "ledger1"));
    }

    #[tokio::test]
    async fn a_rollback_discards_what_a_query_staged() {
        let (_t, mut h) = handler();
        admin_session(&mut h);
        let tx = begin(&mut h);
        let r = handle_transaction_command(
            &mut h,
            tx.clone(),
            query("INSERT {_key: 'q'} INTO c", json!({})),
        )
        .await;
        assert!(!matches!(r, Response::Error { .. }), "{:?}", r);
        assert!(matches!(
            handle_rollback_transaction(&mut h, tx),
            Response::Ok { .. }
        ));
        assert!(
            !exists(&h, "q"),
            "a rolled-back query must not have written"
        );
    }

    /// A query reads committed data, so an `UPDATE` it computes over a
    /// version it did not read (here the transaction's own staged write) is
    /// refused with a conflict instead of overwriting it (card #796).
    #[tokio::test]
    async fn an_update_over_a_version_the_query_did_not_read_is_refused() {
        let (_t, mut h) = handler();
        admin_session(&mut h);
        h.get_collection("d", "c")
            .unwrap()
            .insert(json!({"_key": "a", "n": 10}))
            .unwrap();
        let tx = begin(&mut h);
        let r = handle_transaction_command(
            &mut h,
            tx.clone(),
            Box::new(Command::Update {
                database: "d".to_string(),
                collection: "c".to_string(),
                key: "a".to_string(),
                document: json!({"n": 1}),
                merge: true,
            }),
        )
        .await;
        assert!(!matches!(r, Response::Error { .. }), "{:?}", r);
        let r = handle_transaction_command(
            &mut h,
            tx.clone(),
            query(
                "FOR x IN c FILTER x._key == 'a' UPDATE x WITH {n: x.n + 1} IN c",
                json!({}),
            ),
        )
        .await;
        assert!(
            matches!(&r, Response::Error { error } if error.to_string().contains("conflict")),
            "{:?}",
            r
        );
        handle_rollback_transaction(&mut h, tx);
        assert_eq!(balance(&h, "a"), json!(10));
    }

    fn step(command: Command, require_rows: bool) -> crate::driver::protocol::TransactionStep {
        crate::driver::protocol::TransactionStep {
            command,
            require_rows,
        }
    }

    fn transfer(from: &str, to: &str, x: i64) -> Vec<crate::driver::protocol::TransactionStep> {
        let debit = "FOR a IN c FILTER a._key == @k FILTER a.n >= @x \
                     UPDATE a WITH {n: a.n - @x} IN c RETURN 1";
        let credit = "FOR a IN c FILTER a._key == @k UPDATE a WITH {n: a.n + @x} IN c RETURN 1";
        vec![
            step(*query(debit, json!({"k": from, "x": x})), true),
            step(*query(credit, json!({"k": to, "x": x})), true),
            step(*insert(&format!("ledger-{from}-{to}-{x}")), false),
        ]
    }

    async fn run(
        h: &mut DriverHandler,
        steps: Vec<crate::driver::protocol::TransactionStep>,
    ) -> Response {
        handle_run_transaction(h, "d".to_string(), IsolationLevel::ReadCommitted, steps).await
    }

    fn accounts(h: &DriverHandler) {
        let c = h.get_collection("d", "c").unwrap();
        c.insert(json!({"_key": "a", "n": 100})).unwrap();
        c.insert(json!({"_key": "b", "n": 0})).unwrap();
    }

    #[tokio::test]
    async fn a_run_transaction_commits_every_step() {
        let (_t, mut h) = handler();
        admin_session(&mut h);
        accounts(&h);
        let r = run(&mut h, transfer("a", "b", 30)).await;
        let Response::Ok { data: Some(d), .. } = &r else {
            panic!("{r:?}")
        };
        assert_eq!(d["committed"], json!(true), "{d}");
        assert_eq!(d["results"][0], json!([1]));
        assert_eq!(d["results"][1], json!([1]));
        assert_eq!(balance(&h, "a"), json!(70));
        assert_eq!(balance(&h, "b"), json!(30));
        assert!(exists(&h, "ledger-a-b-30"));
        assert!(h.transactions.is_empty(), "no transaction left open");
    }

    #[tokio::test]
    async fn a_required_step_that_matches_nothing_rolls_back() {
        let (_t, mut h) = handler();
        admin_session(&mut h);
        accounts(&h);
        // b holds 0: the debit matches nothing, the credit and the ledger
        // row are not applied.
        let r = run(&mut h, transfer("b", "a", 5)).await;
        let Response::Ok { data: Some(d), .. } = &r else {
            panic!("{r:?}")
        };
        assert_eq!(d, &json!({"committed": false, "stopped_at": 0}));
        assert_eq!(balance(&h, "a"), json!(100));
        assert!(!exists(&h, "ledger-b-a-5"));
        assert!(h.transactions.is_empty());
    }

    #[tokio::test]
    async fn a_failing_step_rolls_back_and_names_itself() {
        let (_t, mut h) = handler();
        admin_session(&mut h);
        accounts(&h);
        // Updating a document that does not exist fails as it is staged,
        // at step 3.
        let mut steps = transfer("a", "b", 10);
        steps.push(step(
            Command::Update {
                database: "d".to_string(),
                collection: "c".to_string(),
                key: "missing".to_string(),
                document: json!({"n": 1}),
                merge: true,
            },
            false,
        ));
        let r = run(&mut h, steps).await;
        assert!(
            matches!(&r, Response::Error { error } if error.to_string().contains("step 3")),
            "{r:?}"
        );
        // Inserting a key that exists is refused by the commit.
        let mut steps = transfer("a", "b", 10);
        steps.push(step(*insert("a"), false));
        let r = run(&mut h, steps).await;
        assert!(matches!(r, Response::Error { .. }), "{r:?}");
        // Neither left anything behind.
        assert_eq!(balance(&h, "a"), json!(100));
        assert_eq!(balance(&h, "b"), json!(0));
        assert!(!exists(&h, "ledger-a-b-10"));
        assert!(h.transactions.is_empty());
    }

    #[tokio::test]
    async fn a_run_transaction_stays_in_its_database_and_needs_write() {
        let (_t, mut h) = handler();
        admin_session(&mut h);
        accounts(&h);
        let elsewhere = vec![step(
            Command::Insert {
                database: "other".to_string(),
                collection: "c".to_string(),
                key: Some("x".to_string()),
                document: json!({}),
            },
            false,
        )];
        let r = run(&mut h, elsewhere).await;
        assert!(matches!(r, Response::Error { .. }), "{r:?}");

        let (_t2, mut reader) = handler();
        reader.authenticated_db = Some("d".to_string());
        reader
            .session_permissions
            .insert(crate::server::Permission::database_permission(
                crate::server::PermissionAction::Read,
                "d",
            ));
        accounts(&reader);
        let r = run(&mut reader, transfer("a", "b", 1)).await;
        assert!(matches!(r, Response::Error { .. }), "{r:?}");
        assert_eq!(balance(&reader, "a"), json!(100));
        assert!(reader.transactions.is_empty());
    }

    #[test]
    fn point_statements_are_recognised() {
        let point = |q: &str| is_point_statement(&crate::sdbql::parse(q).unwrap());
        assert!(point(
            "FOR a IN c FILTER a._key == @k FILTER a.n >= @x UPDATE a WITH {n: a.n - @x} IN c RETURN 1"
        ));
        assert!(point("FOR a IN c FILTER 'x' == a._key REMOVE a IN c"));
        assert!(point("INSERT {_key: @id, n: 1} INTO c"));
        // A scan, a range, or a loop: the blocking pool.
        assert!(!point(
            "FOR a IN c FILTER a.n > 3 UPDATE a WITH {n: 0} IN c"
        ));
        assert!(!point("FOR a IN c UPDATE a WITH {n: 0} IN c"));
        assert!(!point("FOR i IN 1..10 INSERT {i} INTO c"));
        assert!(!point(
            "FOR a IN c FILTER a._key == a.other UPDATE a WITH {n: 0} IN c"
        ));
    }

    #[tokio::test]
    async fn a_query_that_writes_needs_write_permission() {
        let (_t, mut h) = handler();
        h.authenticated_db = Some("d".to_string());
        h.session_permissions
            .insert(crate::server::Permission::database_permission(
                crate::server::PermissionAction::Read,
                "d",
            ));
        let tx = begin(&mut h);
        let r = handle_transaction_command(
            &mut h,
            tx.clone(),
            query("INSERT {_key: 'q'} INTO c", json!({})),
        )
        .await;
        assert!(matches!(r, Response::Error { .. }), "{:?}", r);
        handle_commit_transaction(&mut h, tx);
        assert!(!exists(&h, "q"));
    }

    #[tokio::test]
    async fn unique_indexes_are_enforced_inside_a_transaction() {
        let (_t, mut h) = handler();
        h.get_collection("d", "c")
            .unwrap()
            .create_index(
                "uniq".to_string(),
                vec!["email".to_string()],
                crate::storage::index::IndexType::Persistent,
                true,
            )
            .unwrap();
        let with_email = |key: &str| {
            Box::new(Command::Insert {
                database: "d".to_string(),
                collection: "c".to_string(),
                key: Some(key.to_string()),
                document: json!({"email": "same@example.com"}),
            })
        };
        let tx = begin(&mut h);
        handle_transaction_command(&mut h, tx.clone(), with_email("u1")).await;
        let second = handle_transaction_command(&mut h, tx.clone(), with_email("u2")).await;
        let commit = handle_commit_transaction(&mut h, tx);

        // Either the second insert or the commit must refuse it, and never
        // both documents may land.
        let refused =
            matches!(second, Response::Error { .. }) || matches!(commit, Response::Error { .. });
        assert!(refused, "second={:?} commit={:?}", second, commit);
        assert!(
            !(exists(&h, "u1") && exists(&h, "u2")),
            "duplicate unique value stored"
        );
    }

    #[tokio::test]
    async fn unsupported_inner_command_is_refused() {
        let (_t, mut h) = handler();
        let tx = begin(&mut h);
        let cmd = Box::new(Command::Get {
            database: "d".to_string(),
            collection: "c".to_string(),
            key: "x".to_string(),
        });
        let r = handle_transaction_command(&mut h, tx, cmd).await;
        assert!(matches!(r, Response::Error { .. }), "{:?}", r);
    }
}
