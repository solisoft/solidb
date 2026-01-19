use crate::driver::protocol::{Command, DriverError, IsolationLevel, Response};
use crate::driver::handlers::DriverHandler;
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
        Some(tx) => match handler.storage.commit_transaction(tx) {
            Ok(_) => Response::ok_empty(),
            Err(e) => Response::error(DriverError::TransactionError(e.to_string())),
        },
        None => Response::error(DriverError::TransactionError(
            "Transaction not found".to_string(),
        )),
    }
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

pub async fn handle_transaction_command(
    handler: &mut DriverHandler,
    tx_id: String,
    command: Box<Command>,
) -> Response {
    // Verify transaction exists
    if !handler.transactions.contains_key(&tx_id) {
        return Response::error(DriverError::TransactionError(
            "Transaction not found".to_string(),
        ));
    }
    // Execute the inner command (transaction context not yet implemented for all commands)
    // For now, just execute the command normally
    // TODO: Implement proper transaction context
    Box::pin(handler.execute_command(*command)).await
}
