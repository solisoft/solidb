use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("Collection '{0}' not found")]
    CollectionNotFound(String),

    #[error("Document with key '{0}' not found")]
    DocumentNotFound(String),

    #[error("Collection '{0}' already exists")]
    CollectionAlreadyExists(String),

    #[error("Invalid document: {0}")]
    InvalidDocument(String),

    #[error("Conflict: {0}")]
    ConflictError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Query execution error: {0}")]
    ExecutionError(String),

    #[error("Bad Request: {0}")]
    BadRequest(String),

    #[error("Operation not supported: {0}")]
    OperationNotSupported(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Internal error: {0}")]
    InternalError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    // Transaction errors
    #[error("Transaction '{0}' not found")]
    TransactionNotFound(String),

    #[error("Transaction conflict: {0}")]
    TransactionConflict(String),

    #[error("Deadlock detected: {0}")]
    DeadlockDetected(String),

    #[error("Transaction timeout: {0}")]
    TransactionTimeout(String),

    #[error("Isolation violation: {0}")]
    IsolationViolation(String),

    // Schema validation errors
    #[error("Schema validation failed: {0}")]
    SchemaValidationError(String),

    #[error("Schema compilation failed: {0}")]
    SchemaCompilationError(String),

    // Authorization errors
    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Role not found: {0}")]
    RoleNotFound(String),
}

pub type DbResult<T> = Result<T, DbError>;

impl serde::Serialize for DbError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

impl From<rocksdb::Error> for DbError {
    fn from(err: rocksdb::Error) -> Self {
        DbError::InternalError(err.into())
    }
}

impl IntoResponse for DbError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            DbError::CollectionNotFound(msg)
            | DbError::DocumentNotFound(msg)
            | DbError::TransactionNotFound(msg)
            | DbError::RoleNotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            DbError::CollectionAlreadyExists(msg)
            | DbError::ConflictError(msg)
            | DbError::TransactionConflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            DbError::InvalidDocument(msg)
            | DbError::ParseError(msg)
            | DbError::BadRequest(msg)
            | DbError::SchemaValidationError(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            DbError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            DbError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            DbError::OperationNotSupported(msg) => (StatusCode::NOT_IMPLEMENTED, msg.clone()),
            DbError::TransactionTimeout(_) => (StatusCode::REQUEST_TIMEOUT, self.to_string()),
            // Default to 500
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        let body = serde_json::json!({
            "error": message,
            "code": status.as_u16(),
            "type": format!("{:?}", self).split('(').next().unwrap_or("Error") // Basic type extraction
        });

        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_messages() {
        let err = DbError::CollectionNotFound("users".to_string());
        assert_eq!(err.to_string(), "Collection 'users' not found");

        let err = DbError::DocumentNotFound("doc123".to_string());
        assert_eq!(err.to_string(), "Document with key 'doc123' not found");

        let err = DbError::CollectionAlreadyExists("users".to_string());
        assert_eq!(err.to_string(), "Collection 'users' already exists");

        let err = DbError::InvalidDocument("missing _key".to_string());
        assert_eq!(err.to_string(), "Invalid document: missing _key");

        let err = DbError::ParseError("unexpected token".to_string());
        assert_eq!(err.to_string(), "Parse error: unexpected token");

        let err = DbError::ExecutionError("division by zero".to_string());
        assert_eq!(err.to_string(), "Query execution error: division by zero");

        let err = DbError::BadRequest("invalid parameter".to_string());
        assert_eq!(err.to_string(), "Bad Request: invalid parameter");

        let err = DbError::OperationNotSupported("bulk delete".to_string());
        assert_eq!(err.to_string(), "Operation not supported: bulk delete");

        let err = DbError::InternalError("storage failure".to_string());
        assert_eq!(err.to_string(), "Internal error: storage failure");

        let err = DbError::NetworkError("connection refused".to_string());
        assert_eq!(err.to_string(), "Network error: connection refused");
    }

    #[test]
    fn test_transaction_errors() {
        let err = DbError::TransactionNotFound("tx123".to_string());
        assert_eq!(err.to_string(), "Transaction 'tx123' not found");

        let err = DbError::TransactionConflict("write-write conflict".to_string());
        assert_eq!(
            err.to_string(),
            "Transaction conflict: write-write conflict"
        );

        let err = DbError::DeadlockDetected("cycle detected".to_string());
        assert_eq!(err.to_string(), "Deadlock detected: cycle detected");

        let err = DbError::TransactionTimeout("exceeded 30s".to_string());
        assert_eq!(err.to_string(), "Transaction timeout: exceeded 30s");

        let err = DbError::IsolationViolation("phantom read".to_string());
        assert_eq!(err.to_string(), "Isolation violation: phantom read");
    }

    #[test]
    fn test_error_debug() {
        let err = DbError::CollectionNotFound("test".to_string());
        let debug = format!("{:?}", err);
        assert!(debug.contains("CollectionNotFound"));
    }

    #[test]
    fn test_db_result_type() {
        let ok_result: DbResult<i32> = Ok(42);
        assert_eq!(ok_result.unwrap(), 42);

        let err_result: DbResult<i32> = Err(DbError::InternalError("test".to_string()));
        assert!(err_result.is_err());
    }
}
