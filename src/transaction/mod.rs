pub mod manager;
pub mod wal;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// Unique identifier for a transaction (timestamp-based for ordering)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TransactionId(u64);

impl TransactionId {
    /// Create a new transaction ID based on current timestamp
    pub fn new() -> Self {
        Self(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64,
        )
    }

    /// Create a transaction ID from a raw value
    pub fn from_u64(id: u64) -> Self {
        Self(id)
    }

    /// Get the raw value
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl Default for TransactionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TransactionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tx:{}", self.0)
    }
}

/// Transaction state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionState {
    /// Transaction is active and accepting operations
    Active,
    /// Transaction is being committed (two-phase commit)
    Preparing,
    /// Transaction has been committed successfully
    Committed,
    /// Transaction has been aborted/rolled back
    Aborted,
}

/// Isolation level for transactions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IsolationLevel {
    /// Read uncommitted data (dirty reads possible)
    ReadUncommitted,
    /// Read only committed data (default)
    ReadCommitted,
    /// Repeatable reads within transaction
    RepeatableRead,
    /// Fully serializable execution
    Serializable,
}

impl Default for IsolationLevel {
    fn default() -> Self {
        Self::ReadCommitted
    }
}

/// Type of operation within a transaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operation {
    /// Insert a document
    Insert {
        database: String,
        collection: String,
        key: String,
        data: Value,
    },
    /// Update a document
    Update {
        database: String,
        collection: String,
        key: String,
        old_data: Value,
        new_data: Value,
    },
    /// Delete a document
    Delete {
        database: String,
        collection: String,
        key: String,
        old_data: Value,
    },
    /// Store a blob chunk
    PutBlobChunk {
        database: String,
        collection: String,
        key: String,
        chunk_index: u32,
        data: Vec<u8>,
    },
    /// Delete blob data
    DeleteBlob {
        database: String,
        collection: String,
        key: String,
    },
}

impl Operation {
    /// Get the database name for this operation
    pub fn database(&self) -> &str {
        match self {
            Operation::Insert { database, .. } => database,
            Operation::Update { database, .. } => database,
            Operation::Delete { database, .. } => database,
            Operation::PutBlobChunk { database, .. } => database,
            Operation::DeleteBlob { database, .. } => database,
        }
    }

    /// Get the collection name for this operation
    pub fn collection(&self) -> &str {
        match self {
            Operation::Insert { collection, .. } => collection,
            Operation::Update { collection, .. } => collection,
            Operation::Delete { collection, .. } => collection,
            Operation::PutBlobChunk { collection, .. } => collection,
            Operation::DeleteBlob { collection, .. } => collection,
        }
    }

    /// Get the document key for this operation
    pub fn key(&self) -> &str {
        match self {
            Operation::Insert { key, .. } => key,
            Operation::Update { key, .. } => key,
            Operation::Delete { key, .. } => key,
            Operation::PutBlobChunk { key, .. } => key,
            Operation::DeleteBlob { key, .. } => key,
        }
    }
}

/// Represents an active transaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    /// Unique transaction identifier
    pub id: TransactionId,
    /// Current state
    pub state: TransactionState,
    /// Isolation level
    pub isolation_level: IsolationLevel,
    /// List of operations performed in this transaction
    pub operations: Vec<Operation>,
    /// Timestamp when transaction started (for MVCC)
    pub read_timestamp: u64,
    /// Timestamp when transaction commits (for MVCC)
    pub write_timestamp: Option<u64>,
    /// When the transaction was created
    pub created_at: DateTime<Utc>,
    /// Validation errors encountered (cleared on successful validation)
    pub validation_errors: Vec<String>,
}

impl Transaction {
    /// Create a new transaction
    pub fn new(isolation_level: IsolationLevel) -> Self {
        let id = TransactionId::new();
        let read_timestamp = id.as_u64();

        Self {
            id,
            state: TransactionState::Active,
            isolation_level,
            operations: Vec::new(),
            read_timestamp,
            write_timestamp: None,
            created_at: Utc::now(),
            validation_errors: Vec::new(),
        }
    }

    /// Add an operation to the transaction
    pub fn add_operation(&mut self, operation: Operation) {
        self.operations.push(operation);
    }

    /// Check if the transaction is active
    pub fn is_active(&self) -> bool {
        self.state == TransactionState::Active
    }

    /// Add a validation error
    pub fn add_validation_error(&mut self, error: String) {
        self.validation_errors.push(error);
    }

    /// Check if transaction has validation errors
    pub fn has_validation_errors(&self) -> bool {
        !self.validation_errors.is_empty()
    }

    /// Get all validation errors
    pub fn get_validation_errors(&self) -> &[String] {
        &self.validation_errors
    }

    /// Clear validation errors
    pub fn clear_validation_errors(&mut self) {
        self.validation_errors.clear();
    }

    /// Mark transaction as preparing to commit
    pub fn prepare(&mut self) {
        self.state = TransactionState::Preparing;
        self.write_timestamp = Some(TransactionId::new().as_u64());
    }

    /// Mark transaction as committed
    pub fn commit(&mut self) {
        self.state = TransactionState::Committed;
    }

    /// Mark transaction as aborted
    pub fn abort(&mut self) {
        self.state = TransactionState::Aborted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_id_ordering() {
        let id1 = TransactionId::new();
        std::thread::sleep(std::time::Duration::from_nanos(100));
        let id2 = TransactionId::new();
        assert!(id1 < id2);
    }

    #[test]
    fn test_transaction_lifecycle() {
        let mut tx = Transaction::new(IsolationLevel::ReadCommitted);
        assert_eq!(tx.state, TransactionState::Active);
        assert!(tx.is_active());

        tx.prepare();
        assert_eq!(tx.state, TransactionState::Preparing);
        assert!(!tx.is_active());
        assert!(tx.write_timestamp.is_some());

        tx.commit();
        assert_eq!(tx.state, TransactionState::Committed);
    }

    #[test]
    fn test_transaction_operations() {
        let mut tx = Transaction::new(IsolationLevel::ReadCommitted);
        
        tx.add_operation(Operation::Insert {
            database: "_system".to_string(),
            collection: "users".to_string(),
            key: "user1".to_string(),
            data: serde_json::json!({"name": "Alice"}),
        });

        assert_eq!(tx.operations.len(), 1);
        assert_eq!(tx.operations[0].database(), "_system");
        assert_eq!(tx.operations[0].collection(), "users");
        assert_eq!(tx.operations[0].key(), "user1");
    }

    #[test]
    fn test_isolation_level_default() {
        let level = IsolationLevel::default();
        assert_eq!(level, IsolationLevel::ReadCommitted);
    }
}
