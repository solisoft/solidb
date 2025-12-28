//! Transaction Module Unit Tests
//!
//! Tests for the ACID transaction system, covering:
//! - Transaction lifecycle (begin, commit, rollback)
//! - Operation tracking
//! - Transaction manager
//! - Isolation levels

use solidb::transaction::{Transaction, TransactionId, TransactionState, IsolationLevel, Operation};
use solidb::transaction::manager::TransactionManager;
use serde_json::json;
use tempfile::TempDir;
use std::path::PathBuf;

// ============================================================================
// TransactionId Tests
// ============================================================================

#[test]
fn test_transaction_id_new() {
    let id = TransactionId::new();
    assert!(id.as_u64() > 0, "Transaction ID should be non-zero");
}

#[test]
fn test_transaction_id_ordering() {
    let id1 = TransactionId::new();
    std::thread::sleep(std::time::Duration::from_nanos(100));
    let id2 = TransactionId::new();
    
    assert!(id1 < id2, "Later transaction should have higher ID");
}

#[test]
fn test_transaction_id_from_u64() {
    let raw_id = 12345u64;
    let id = TransactionId::from_u64(raw_id);
    assert_eq!(id.as_u64(), raw_id);
}

#[test]
fn test_transaction_id_display() {
    let id = TransactionId::from_u64(1000);
    let display = format!("{}", id);
    assert!(display.contains("tx:"), "Display should include tx: prefix");
    assert!(display.contains("1000"), "Display should include the value");
}

// ============================================================================
// Transaction Lifecycle Tests
// ============================================================================

#[test]
fn test_transaction_new() {
    let tx = Transaction::new(IsolationLevel::ReadCommitted);
    
    assert!(tx.is_active(), "New transaction should be active");
    assert_eq!(tx.state, TransactionState::Active);
    assert!(tx.operations.is_empty(), "New transaction should have no operations");
}

#[test]
fn test_transaction_add_operation() {
    let mut tx = Transaction::new(IsolationLevel::ReadCommitted);
    
    tx.add_operation(Operation::Insert {
        database: "_system".to_string(),
        collection: "users".to_string(),
        key: "user1".to_string(),
        data: json!({"name": "Alice"}),
    });
    
    assert_eq!(tx.operations.len(), 1);
}

#[test]
fn test_transaction_prepare() {
    let mut tx = Transaction::new(IsolationLevel::ReadCommitted);
    assert!(tx.write_timestamp.is_none());
    
    tx.prepare();
    
    assert_eq!(tx.state, TransactionState::Preparing);
    assert!(!tx.is_active());
    assert!(tx.write_timestamp.is_some(), "Prepare should set write timestamp");
}

#[test]
fn test_transaction_commit() {
    let mut tx = Transaction::new(IsolationLevel::ReadCommitted);
    tx.prepare();
    tx.commit();
    
    assert_eq!(tx.state, TransactionState::Committed);
}

#[test]
fn test_transaction_abort() {
    let mut tx = Transaction::new(IsolationLevel::ReadCommitted);
    tx.abort();
    
    assert_eq!(tx.state, TransactionState::Aborted);
    assert!(!tx.is_active());
}

// ============================================================================
// Operation Tests  
// ============================================================================

#[test]
fn test_operation_insert_accessors() {
    let op = Operation::Insert {
        database: "mydb".to_string(),
        collection: "users".to_string(),
        key: "doc1".to_string(),
        data: json!({"field": "value"}),
    };
    
    assert_eq!(op.database(), "mydb");
    assert_eq!(op.collection(), "users");
    assert_eq!(op.key(), "doc1");
}

#[test]
fn test_operation_update_accessors() {
    let op = Operation::Update {
        database: "testdb".to_string(),
        collection: "items".to_string(),
        key: "item1".to_string(),
        old_data: json!({"value": 1}),
        new_data: json!({"value": 2}),
    };
    
    assert_eq!(op.database(), "testdb");
    assert_eq!(op.collection(), "items");
    assert_eq!(op.key(), "item1");
}

#[test]
fn test_operation_delete_accessors() {
    let op = Operation::Delete {
        database: "app".to_string(),
        collection: "sessions".to_string(),
        key: "session123".to_string(),
        old_data: json!({"expired": true}),
    };
    
    assert_eq!(op.database(), "app");
    assert_eq!(op.collection(), "sessions");
    assert_eq!(op.key(), "session123");
}

#[test]
fn test_operation_blob_accessors() {
    let op = Operation::PutBlobChunk {
        database: "files".to_string(),
        collection: "uploads".to_string(),
        key: "file1".to_string(),
        chunk_index: 0,
        data: vec![1, 2, 3, 4],
    };
    
    assert_eq!(op.database(), "files");
    assert_eq!(op.collection(), "uploads");
    assert_eq!(op.key(), "file1");
    
    let op2 = Operation::DeleteBlob {
        database: "files".to_string(),
        collection: "uploads".to_string(),
        key: "file2".to_string(),
    };
    
    assert_eq!(op2.database(), "files");
    assert_eq!(op2.collection(), "uploads");
    assert_eq!(op2.key(), "file2");
}

// ============================================================================
// Isolation Level Tests
// ============================================================================

#[test]
fn test_isolation_level_default() {
    let level = IsolationLevel::default();
    assert_eq!(level, IsolationLevel::ReadCommitted);
}

#[test]
fn test_isolation_level_variants() {
    let levels = [
        IsolationLevel::ReadUncommitted,
        IsolationLevel::ReadCommitted,
        IsolationLevel::RepeatableRead,
        IsolationLevel::Serializable,
    ];
    
    // Verify all variants can be created and compared
    for level in &levels {
        let tx = Transaction::new(*level);
        assert_eq!(tx.isolation_level, *level);
    }
}

// ============================================================================
// Validation Error Tests
// ============================================================================

#[test]
fn test_transaction_validation_errors() {
    let mut tx = Transaction::new(IsolationLevel::ReadCommitted);
    
    assert!(!tx.has_validation_errors());
    assert!(tx.get_validation_errors().is_empty());
    
    tx.add_validation_error("Unique constraint violation".to_string());
    tx.add_validation_error("Foreign key constraint".to_string());
    
    assert!(tx.has_validation_errors());
    assert_eq!(tx.get_validation_errors().len(), 2);
}

#[test]
fn test_transaction_clear_validation_errors() {
    let mut tx = Transaction::new(IsolationLevel::ReadCommitted);
    tx.add_validation_error("Error 1".to_string());
    
    assert!(tx.has_validation_errors());
    
    tx.clear_validation_errors();
    
    assert!(!tx.has_validation_errors());
}

// ============================================================================
// Transaction Manager Tests
// ============================================================================

fn create_test_tx_manager() -> (TransactionManager, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let wal_path = tmp_dir.path().join("wal");
    let manager = TransactionManager::new(wal_path)
        .expect("Failed to create transaction manager");
    (manager, tmp_dir)
}

#[test]
fn test_transaction_manager_begin() {
    let (manager, _tmp) = create_test_tx_manager();
    
    let tx_id = manager.begin(IsolationLevel::ReadCommitted);
    assert!(tx_id.is_ok(), "Should begin transaction: {:?}", tx_id.err());
    
    let tx_id = tx_id.unwrap();
    assert!(manager.is_active(tx_id), "Transaction should be active");
}

#[test]
fn test_transaction_manager_commit() {
    let (manager, _tmp) = create_test_tx_manager();
    
    let tx_id = manager.begin(IsolationLevel::ReadCommitted).unwrap();
    
    let result = manager.commit(tx_id);
    assert!(result.is_ok(), "Should commit transaction: {:?}", result.err());
    
    // Transaction should no longer be active after commit
    assert!(!manager.is_active(tx_id));
}

#[test]
fn test_transaction_manager_rollback() {
    let (manager, _tmp) = create_test_tx_manager();
    
    let tx_id = manager.begin(IsolationLevel::ReadCommitted).unwrap();
    
    let result = manager.rollback(tx_id);
    assert!(result.is_ok(), "Should rollback transaction: {:?}", result.err());
    
    assert!(!manager.is_active(tx_id));
}

#[test]
fn test_transaction_manager_multiple_transactions() {
    let (manager, _tmp) = create_test_tx_manager();
    
    let tx1 = manager.begin(IsolationLevel::ReadCommitted).unwrap();
    let tx2 = manager.begin(IsolationLevel::ReadCommitted).unwrap();
    let tx3 = manager.begin(IsolationLevel::ReadCommitted).unwrap();
    
    // All should be different
    assert_ne!(tx1, tx2);
    assert_ne!(tx2, tx3);
    
    // All should be active
    assert!(manager.is_active(tx1));
    assert!(manager.is_active(tx2));
    assert!(manager.is_active(tx3));
    
    // Transaction count
    assert_eq!(manager.transaction_count(), 3);
    
    // Commit one, rollback one
    manager.commit(tx1).unwrap();
    manager.rollback(tx2).unwrap();
    
    assert!(!manager.is_active(tx1));
    assert!(!manager.is_active(tx2));
    assert!(manager.is_active(tx3));
}

#[test]
fn test_transaction_manager_get_transaction() {
    let (manager, _tmp) = create_test_tx_manager();
    
    let tx_id = manager.begin(IsolationLevel::Serializable).unwrap();
    
    let tx = manager.get(tx_id);
    assert!(tx.is_ok());
    
    let tx = tx.unwrap();
    let tx_guard = tx.read().unwrap();
    assert_eq!(tx_guard.isolation_level, IsolationLevel::Serializable);
}

#[test]
fn test_transaction_manager_nonexistent_transaction() {
    let (manager, _tmp) = create_test_tx_manager();
    
    let fake_id = TransactionId::from_u64(999999);
    
    let result = manager.get(fake_id);
    assert!(result.is_err(), "Should fail for nonexistent transaction");
}

// ============================================================================
// Transaction with Operations Tests
// ============================================================================

#[test]
fn test_transaction_multiple_operations() {
    let mut tx = Transaction::new(IsolationLevel::ReadCommitted);
    
    // Insert operation
    tx.add_operation(Operation::Insert {
        database: "_system".to_string(),
        collection: "users".to_string(),
        key: "user1".to_string(),
        data: json!({"name": "Alice"}),
    });
    
    // Update operation
    tx.add_operation(Operation::Update {
        database: "_system".to_string(),
        collection: "users".to_string(),
        key: "user1".to_string(),
        old_data: json!({"name": "Alice"}),
        new_data: json!({"name": "Alice", "verified": true}),
    });
    
    // Delete operation
    tx.add_operation(Operation::Delete {
        database: "_system".to_string(),
        collection: "logs".to_string(),
        key: "log1".to_string(),
        old_data: json!({"message": "temp"}),
    });
    
    assert_eq!(tx.operations.len(), 3);
    
    // Verify operations maintained their order
    matches!(&tx.operations[0], Operation::Insert { .. });
    matches!(&tx.operations[1], Operation::Update { .. });
    matches!(&tx.operations[2], Operation::Delete { .. });
}

#[test]
fn test_transaction_read_timestamp() {
    let tx = Transaction::new(IsolationLevel::ReadCommitted);
    
    // Read timestamp should be set on creation
    assert!(tx.read_timestamp > 0, "Read timestamp should be set");
    
    // Should match transaction ID
    assert_eq!(tx.read_timestamp, tx.id.as_u64());
}
