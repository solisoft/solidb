//! Error types for sdbql-core.
//!
//! Minimal error types without server dependencies (no axum, no rocksdb).

use thiserror::Error;

/// SDBQL error type
#[derive(Error, Debug)]
pub enum SdbqlError {
    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Execution error: {0}")]
    ExecutionError(String),

    #[error("Collection not found: {0}")]
    CollectionNotFound(String),

    #[error("Operation not supported: {0}")]
    OperationNotSupported(String),

    #[error("Type error: {0}")]
    TypeError(String),
}

/// Result type for SDBQL operations
pub type SdbqlResult<T> = Result<T, SdbqlError>;

impl serde::Serialize for SdbqlError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_messages() {
        let err = SdbqlError::ParseError("unexpected token".to_string());
        assert_eq!(err.to_string(), "Parse error: unexpected token");

        let err = SdbqlError::ExecutionError("division by zero".to_string());
        assert_eq!(err.to_string(), "Execution error: division by zero");

        let err = SdbqlError::CollectionNotFound("users".to_string());
        assert_eq!(err.to_string(), "Collection not found: users");

        let err = SdbqlError::OperationNotSupported("INSERT".to_string());
        assert_eq!(err.to_string(), "Operation not supported: INSERT");

        let err = SdbqlError::TypeError("expected number".to_string());
        assert_eq!(err.to_string(), "Type error: expected number");
    }

    #[test]
    fn test_result_type() {
        let ok_result: SdbqlResult<i32> = Ok(42);
        assert_eq!(ok_result.unwrap(), 42);

        let err_result: SdbqlResult<i32> = Err(SdbqlError::ExecutionError("test".to_string()));
        assert!(err_result.is_err());
    }
}
