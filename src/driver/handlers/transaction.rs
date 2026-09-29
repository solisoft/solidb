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
            match handler.storage.commit_transaction(tx) {
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
/// Only single-document writes (`Insert`, `Update`, `Delete`) can be staged in
/// a transaction: they are recorded on it and applied at commit, so a
/// rollback discards them. Anything else is refused. It used to run the inner
/// command outside the transaction, which committed the write immediately and
/// made `Rollback` a no-op without saying so.
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
        _ => Response::error(DriverError::TransactionError(
            "only Insert, Update and Delete can run inside a transaction".to_string(),
        )),
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
