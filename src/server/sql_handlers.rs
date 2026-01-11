use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use super::handlers::AppState;
use crate::sql::translate_sql_to_sdbql;

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

    // Create executor with database context and bind variables
    let executor = crate::sdbql::QueryExecutor::with_database_and_bind_vars(
        &state.storage,
        db.clone(),
        request.bind_vars,
    );

    // Execute the query
    match executor.execute(&query_ast) {
        Ok(results) => Ok(Json(SqlResponse {
            result: Value::Array(results),
            sdbql: Some(sdbql),
            error: None,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SqlResponse {
                result: Value::Null,
                sdbql: Some(sdbql),
                error: Some(format!("Query execution error: {}", e)),
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
