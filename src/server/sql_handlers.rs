use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use super::handlers::query::{invalidate_collections, mutated_collections};
use super::handlers::AppState;
use crate::sql::translate_sql_to_sdbql;

/// Execution timeout for `/sql`, the same bound `/cursor` applies.
const SQL_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Deserialize)]
pub struct SqlRequest {
    pub query: String,
    #[serde(default)]
    pub bind_vars: HashMap<String, Value>,
    /// If true, return the translated SDBQL instead of executing
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct SqlResponse {
    pub result: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdbql: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SqlTranslateResponse {
    pub sdbql: String,
    pub sql: String,
}

/// Execute a SQL query by translating to SDBQL and running it
pub async fn execute_sql_handler(
    State(state): State<AppState>,
    Path(db): Path<String>,
    axum::Extension(claims): axum::Extension<crate::server::auth::Claims>,
    Json(request): Json<SqlRequest>,
) -> Result<Json<SqlResponse>, (StatusCode, Json<SqlResponse>)> {
    // Translate SQL to SDBQL
    let sdbql = match translate_sql_to_sdbql(&request.query) {
        Ok(s) => s,
        Err(e) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(SqlResponse {
                    result: Value::Null,
                    sdbql: None,
                    error: Some(format!("SQL parse error: {}", e)),
                }),
            ));
        }
    };

    // If dry_run, just return the translated SDBQL
    if request.dry_run {
        return Ok(Json(SqlResponse {
            result: Value::Null,
            sdbql: Some(sdbql),
            error: None,
        }));
    }

    // Parse the SDBQL string into a Query AST
    let query_ast = match crate::sdbql::parse(&sdbql) {
        Ok(ast) => ast,
        Err(e) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(SqlResponse {
                    result: Value::Null,
                    sdbql: Some(sdbql),
                    error: Some(format!("SDBQL parse error: {}", e)),
                }),
            ));
        }
    };

    // The authz middleware only required Read for /sql; translated SQL can
    // mutate (INSERT/UPDATE/DELETE), so upgrade to Write when it does.
    if query_ast.has_mutations() {
        if let Err(e) = crate::server::authz_middleware::enforce(
            &claims,
            &state,
            crate::server::authorization::PermissionAction::Write,
            Some(&db),
        )
        .await
        {
            return Err((
                StatusCode::FORBIDDEN,
                Json(SqlResponse {
                    result: Value::Null,
                    sdbql: Some(sdbql),
                    error: Some(e.to_string()),
                }),
            ));
        }
    }

    // Audit P1: mirror `/cursor` — the executor is synchronous CPU work, so it
    // runs on the blocking pool under the same timeout instead of pinning an
    // async worker, and a mutation invalidates the result cache (or `/cursor`
    // keeps serving the pre-write rows).
    let mutates = query_ast.has_mutations();
    let invalidated: Vec<String> = if mutates {
        mutated_collections(&query_ast).into_iter().collect()
    } else {
        Vec::new()
    };
    let storage = state.storage.clone();
    let replication_log = state.replication_log.clone();
    let bind_vars = request.bind_vars;
    // `/sql` is a Read route: the principal is what stops a viewer from
    // reaching the write-side query paths (auto-index creation).
    let principal = crate::server::handlers::query::principal_from_claims(&claims);
    let db_name = db;

    let mut task = tokio::task::spawn_blocking(move || {
        let mut executor =
            crate::sdbql::QueryExecutor::with_database_and_bind_vars(&storage, db_name, bind_vars)
                .with_principal(principal)
                .with_timeout(std::time::Duration::from_secs(SQL_TIMEOUT_SECS));
        // Mutating SQL must reach the replication log like every other write path.
        if let Some(ref log) = replication_log {
            executor = executor.with_replication(log);
        }
        executor.execute(&query_ast)
    });

    // `&mut task` so the handle survives a timeout and can still be awaited.
    let outcome =
        match tokio::time::timeout(std::time::Duration::from_secs(SQL_TIMEOUT_SECS), &mut task)
            .await
        {
            Ok(Ok(result)) => result.map_err(|e| format!("Query execution error: {}", e)),
            Ok(Err(e)) => Err(format!("Task join error: {}", e)),
            Err(_) => {
                // A blocking task cannot be cancelled: an overrunning mutation
                // still commits. Drop cached rows now and again once it lands.
                if mutates {
                    invalidate_collections(&invalidated);
                    tokio::spawn(async move {
                        let _ = task.await;
                        invalidate_collections(&invalidated);
                    });
                }
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(SqlResponse {
                        result: Value::Null,
                        sdbql: Some(sdbql),
                        error: Some(format!(
                            "Query execution timeout: exceeded {} seconds",
                            SQL_TIMEOUT_SECS
                        )),
                    }),
                ));
            }
        };

    // Invalidate even on error: a mutation can fail part-way after writing.
    if mutates {
        invalidate_collections(&invalidated);
    }

    match outcome {
        Ok(results) => Ok(Json(SqlResponse {
            result: Value::Array(results),
            sdbql: Some(sdbql),
            error: None,
        })),
        Err(error) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SqlResponse {
                result: Value::Null,
                sdbql: Some(sdbql),
                error: Some(error),
            }),
        )),
    }
}

/// Translate SQL to SDBQL without executing
pub async fn translate_sql_handler(
    Json(request): Json<SqlRequest>,
) -> Result<Json<SqlTranslateResponse>, (StatusCode, Json<SqlResponse>)> {
    match translate_sql_to_sdbql(&request.query) {
        Ok(sdbql) => Ok(Json(SqlTranslateResponse {
            sdbql,
            sql: request.query,
        })),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(SqlResponse {
                result: Value::Null,
                sdbql: None,
                error: Some(format!("SQL parse error: {}", e)),
            }),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sql_translate() {
        let result = translate_sql_to_sdbql("SELECT * FROM users").unwrap();
        assert!(result.contains("FOR doc IN users"));
        assert!(result.contains("RETURN doc"));
    }

    #[test]
    fn test_sql_with_where() {
        let result = translate_sql_to_sdbql("SELECT * FROM users WHERE age > 18").unwrap();
        assert!(result.contains("FILTER doc.age > 18"));
    }
}
