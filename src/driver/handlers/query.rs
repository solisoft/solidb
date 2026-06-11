use crate::driver::protocol::{DriverError, Response};
use crate::driver::DriverHandler;
use crate::sdbql::QueryExecutor;
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
            // The dispatch check only required Read for Query; upgrade to
            // Write when the parsed query mutates (same as the HTTP /cursor).
            if query.has_mutations() {
                if let Err(e) = crate::server::AuthorizationService::check_permission_raw(
                    &handler.session_permissions,
                    crate::server::PermissionAction::Write,
                    Some(&database),
                    handler.session_scoped_databases.as_deref(),
                ) {
                    return Response::error(DriverError::AuthError(e.to_string()));
                }
            }

            // Create executor with database and bind vars
            let mut executor = if bind_vars.is_empty() {
                QueryExecutor::with_database(&handler.storage, database)
            } else {
                QueryExecutor::with_database_and_bind_vars(&handler.storage, database, bind_vars)
            };
            // Mutating queries must reach the replication log, same as the
            // HTTP query handler.
            if let Some(ref log) = handler.replication {
                executor = executor.with_replication(log);
            }

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
