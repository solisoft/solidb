use solidb::{DbResult, StorageEngine, Transaction, TransactionManager, IsolationLevel};
use tempfile::TempDir;

/// Create a test storage engine with transaction support
fn create_test_engine() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(temp_dir.path()).unwrap();
    engine.initialize().unwrap();
    engine.initialize_transactions().unwrap();
    
    // Create a test collection
    let db = engine.get_database("_system").unwrap();
    db.create_collection("users".to_string(), None).unwrap();
    
    (engine, temp_dir)
}

#[test]
fn test_atomic_insert() {
    let (mut engine, _dir) = create_test_engine();
    
    // Begin transaction
    let tx_manager = engine.transaction_manager().unwrap();
    let tx_id = tx_manager.begin(IsolationLevel::ReadCommitted).unwrap();
    
    // Get transaction and perform insert
    {
        let tx_arc = tx_manager.get(tx_id).unwrap();
        let mut tx = tx_arc.write().unwrap();
        let wal = tx_manager.wal();
        
        let collection = engine.get_collection("_system:users").unwrap();
        let doc = serde_json::json!({"name": "Alice", "age": 30});
        collection.insert_tx(&mut tx, wal, doc).unwrap();
    }
    
    // Verify document is NOT in collection before commit
    let collection = engine.get_collection("_system:users").unwrap();
    assert_eq!(collection.count(), 0);
    
    // Commit transaction
    engine.commit_transaction(tx_id).unwrap();
    
    // Verify document IS in collection after commit
    assert_eq!(collection.count(), 1);
    let docs = collection.all();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].data["name"], "Alice");
}

#[test]
fn test_atomic_rollback() {
    let (mut engine, _dir) = create_test_engine();
    
    // Begin transaction
    let tx_manager = engine.transaction_manager().unwrap();
    let tx_id = tx_manager.begin(IsolationLevel::ReadCommitted).unwrap();
    
    // Perform insert
    {
        let tx_arc = tx_manager.get(tx_id).unwrap();
        let mut tx = tx_arc.write().unwrap();
        let wal = tx_manager.wal();
        
        let collection = engine.get_collection("_system:users").unwrap();
        let doc = serde_json::json!({"name": "Bob", "age": 25});
        collection.insert_tx(&mut tx, wal, doc).unwrap();
    }
    
    // Rollback transaction
    engine.rollback_transaction(tx_id).unwrap();
    
    // Verify document is NOT in collection
    let collection = engine.get_collection("_system:users").unwrap();
    assert_eq!(collection.count(), 0);
}

#[test]
fn test_multi_operation_transaction() {
    let (mut engine, _dir) = create_test_engine();
    
    // Insert a document first (non-transactionally)
    let collection = engine.get_collection("_system:users").unwrap();
    let doc1 = serde_json::json!({"_key": "user1", "name": "Alice", "age": 30});
    collection.insert(doc1).unwrap();
    assert_eq!(collection.count(), 1);
    
    // Begin transaction with multiple operations
    let tx_manager = engine.transaction_manager().unwrap();
    let tx_id = tx_manager.begin(IsolationLevel::ReadCommitted).unwrap();
    
    {
        let tx_arc = tx_manager.get(tx_id).unwrap();
        let mut tx = tx_arc.write().unwrap();
        let wal = tx_manager.wal();
        
        // Insert
        let doc2 = serde_json::json!({"_key": "user2", "name": "Bob", "age": 25});
        collection.insert_tx(&mut tx, wal, doc2).unwrap();
        
        // Update
        let update_data = serde_json::json!({"age": 31});
        collection.update_tx(&mut tx, wal, "user1", update_data).unwrap();
        
        // Insert another
        let doc3 = serde_json::json!({"_key": "user3", "name": "Charlie", "age": 35});
        collection.insert_tx(&mut tx, wal, doc3).unwrap();
    }
    
    // Before commit: only original document exists
    assert_eq!(collection.count(), 1);
    
    // Commit transaction
    engine.commit_transaction(tx_id).unwrap();
    
    // After commit: all operations applied
   assert_eq!(collection.count(), 3);
    
    // Verify update
    let user1 = collection.get("user1").unwrap();
    assert_eq!(user1.data["age"], 31);
    
    // Verify inserts
    let user2 = collection.get("user2").unwrap();
    assert_eq!(user2.data["name"], "Bob");
    
    let user3 = collection.get("user3").unwrap();
    assert_eq!(user3.data["name"], "Charlie");
}

#[test]
fn test_transaction_delete() {
    let (mut engine, _dir) = create_test_engine();
    
    // Insert documents (non-transactionally)
    let collection = engine.get_collection("_system:users").unwrap();
    collection.insert(serde_json::json!({"_key": "user1", "name": "Alice"})).unwrap();
    collection.insert(serde_json::json!({"_key": "user2", "name": "Bob"})).unwrap();
    assert_eq!(collection.count(), 2);
    
    // Begin transaction and delete one
    let tx_manager = engine.transaction_manager().unwrap();
    let tx_id = tx_manager.begin(IsolationLevel::ReadCommitted).unwrap();
    
    {
        let tx_arc = tx_manager.get(tx_id).unwrap();
        let mut tx = tx_arc.write().unwrap();
        let wal = tx_manager.wal();
        
        collection.delete_tx(&mut tx, wal, "user1").unwrap();
    }
    
    // Before commit: both documents still exist
    assert_eq!(collection.count(), 2);
    
    // Commit
    engine.commit_transaction(tx_id).unwrap();
    
    // After commit: one document deleted
    assert_eq!(collection.count(), 1);
    assert!(collection.get("user1").is_err());
    assert!(collection.get("user2").is_ok());
}

#[test]
fn test_wal_recovery() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().to_path_buf();
    
    // Create engine, perform transaction, and commit
    {
        let engine = StorageEngine::new(&path).unwrap();
        engine.initialize().unwrap();
        engine.initialize_transactions().unwrap();
        
        let db = engine.get_database("_system").unwrap();
        db.create_collection("users".to_string(), None).unwrap();
        
        let tx_manager = engine.transaction_manager().unwrap();
        let tx_id = tx_manager.begin(IsolationLevel::ReadCommitted).unwrap();
        
        {
            let tx_arc = tx_manager.get(tx_id).unwrap();
            let mut tx = tx_arc.write().unwrap();
            let wal = tx_manager.wal();
            
            let collection = engine.get_collection("_system:users").unwrap();
            let doc = serde_json::json!({"name": "Recovery Test", "value": 42});
            collection.insert_tx(&mut tx, wal, doc).unwrap();
        }
        
        engine.commit_transaction(tx_id).unwrap();
    } // Engine dropped, simulating restart
    
    // Reopen engine - should recover committed transaction
    {
        let engine = StorageEngine::new(&path).unwrap();
        engine.initialize().unwrap();
        engine.initialize_transactions().unwrap();
        
        let collection = engine.get_collection("_system:users").unwrap();
        
        // Document should be recovered
        assert_eq!(collection.count(), 1);
        let docs = collection.all();
        assert_eq!(docs[0].data["name"], "Recovery Test");
        assert_eq!(docs[0].data["value"], 42);
    }
}
