use super::DriverHandler;
use crate::sdbql::QueryExecutor;
use solidb_client::protocol::{DriverError, Response};
use std::collections::HashMap;

pub fn handle_query(
    handler: &DriverHandler,
    database: String,
    sdbql: String,
    bind_vars: Option<HashMap<String, serde_json::Value>>,
) -> Response {
    let bind_vars = bind_vars.unwrap_or_default();
    // Parse the SDBQL query first
    match crate::sdbql::parse(&sdbql) {
        Ok(query) => {
            // Create executor with database and bind vars
            let executor = if bind_vars.is_empty() {
                QueryExecutor::with_database(&handler.storage, database)
            } else {
                QueryExecutor::with_database_and_bind_vars(&handler.storage, database, bind_vars)
            };

            match executor.execute(&query) {
                Ok(results) => Response::ok(serde_json::json!(results)),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(DriverError::DatabaseError(format!("Parse error: {}", e))),
    }
}

pub fn handle_explain(
    handler: &DriverHandler,
    database: String,
    sdbql: String,
    bind_vars: Option<HashMap<String, serde_json::Value>>,
) -> Response {
    let bind_vars = bind_vars.unwrap_or_default();
    // Parse the SDBQL query first
    match crate::sdbql::parse(&sdbql) {
        Ok(query) => {
            let executor = if bind_vars.is_empty() {
                QueryExecutor::with_database(&handler.storage, database)
            } else {
                QueryExecutor::with_database_and_bind_vars(&handler.storage, database, bind_vars)
            };

            match executor.explain(&query) {
                Ok(explanation) => {
                    Response::ok(serde_json::to_value(explanation).unwrap_or_default())
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(DriverError::DatabaseError(format!("Parse error: {}", e))),
    }
}
