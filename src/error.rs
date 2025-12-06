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

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Internal error: {0}")]
    InternalError(String),

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
}

pub type DbResult<T> = Result<T, DbError>;
