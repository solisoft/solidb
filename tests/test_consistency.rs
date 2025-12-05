use solidb::{StorageEngine, IsolationLevel};
use tempfile::TempDir;

/// Create a test storage engine with transaction support
fn create_test_engine() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let mut engine = StorageEngine::new(temp_dir.path()).unwrap();
    engine.initialize().unwrap();
    engine.initialize_transactions().unwrap();
    
    // Create a test collection
    let db = engine.get_database("_system").unwrap();
    db.create_collection("users".to_string()).unwrap();
    
    (engine, temp_dir)
}

#[test]
fn test_duplicate_insert_validation() {
    let (mut engine, _dir) = create_test_engine();
    
    // Begin transaction
    let tx_manager = engine.transaction_manager().unwrap();
    let tx_id = tx_manager.begin(IsolationLevel::ReadCommitted).unwrap();
    
    // Try to insert same key twice
    {
        let tx_arc = tx_manager.get(tx_id).unwrap();
        let mut tx = tx_arc.write().unwrap();
        let wal = tx_manager.wal();
        
        let collection = engine.get_collection("_system:users").unwrap();
        
        // First insert - OK
        let doc1 = serde_json::json!({"_key": "user1", "name": "Alice"});
        collection.insert_tx(&mut tx, wal, doc1).unwrap();
        
        // Second insert with same key - should be caught by validation
        let doc2 = serde_json::json!({"_key": "user1", "name": "Bob"});
        collection.insert_tx(&mut tx, wal, doc2).unwrap();
    }
    
    // Commit should fail due to validation error
    let result = engine.commit_transaction(tx_id);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Duplicate insert"));
    
    // Verify transaction was rolled back - no documents inserted
    let collection = engine.get_collection("_system:users").unwrap();
    assert_eq!(collection.count(), 0);
}

#[test]
fn test_update_after_delete_validation() {
    let (mut engine, _dir) = create_test_engine();
    
    // First, insert a document non-transactionally
    let collection = engine.get_collection("_system:users").unwrap();
    collection.insert(serde_json::json!({"_key": "user1", "name": "Alice"})).unwrap();
    
    // Begin transaction
    let tx_manager = engine.transaction_manager().unwrap();
    let tx_id = tx_manager.begin(IsolationLevel::ReadCommitted).unwrap();
    
    // Try to delete then update the same document
    {
        let tx_arc = tx_manager.get(tx_id).unwrap();
        let mut tx = tx_arc.write().unwrap();
        let wal = tx_manager.wal();
        
        // Delete
        collection.delete_tx(&mut tx, wal, "user1").unwrap();
        
        // Try to update after delete - should be caught by validation
        let update_data = serde_json::json!({"name": "Bob"});
        collection.update_tx(&mut tx, wal, "user1", update_data).unwrap();
    }
    
    // Commit should fail
    let result = engine.commit_transaction(tx_id);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("update deleted document"));
    
    // Original document should still exist (transaction rolled back)
    assert_eq!(collection.count(), 1);
    let doc = collection.get("user1").unwrap();
    assert_eq!(doc.data["name"], "Alice");
}

#[test]
fn test_valid_transaction_passes_validation() {
    let (mut engine, _dir) = create_test_engine();
    
    // Insert initial document
    let collection = engine.get_collection("_system:users").unwrap();
    collection.insert(serde_json::json!({"_key": "user1", "name": "Alice"})).unwrap();
    
    // Begin transaction with valid operations
    let tx_manager = engine.transaction_manager().unwrap();
    let tx_id = tx_manager.begin(IsolationLevel::ReadCommitted).unwrap();
    
    {
        let tx_arc = tx_manager.get(tx_id).unwrap();
        let mut tx = tx_arc.write().unwrap();
        let wal = tx_manager.wal();
        
        // Insert new document
        collection.insert_tx(&mut tx, wal, serde_json::json!({"_key": "user2", "name": "Bob"})).unwrap();
        
        // Update existing document
        collection.update_tx(&mut tx, wal, "user1", serde_json::json!({"age": 30})).unwrap();
        
        // Delete and re-insert is NOT allowed in same transaction
        // But delete alone is fine
        collection.delete_tx(&mut tx, wal, "user1").unwrap();
    }
    
    // Commit should succeed - all operations are valid
    engine.commit_transaction(tx_id).unwrap();
    
    // Verify results
    assert_eq!(collection.count(), 1);  // user1 deleted, user2 inserted
    assert!(collection.get("user1").is_err());
    assert!(collection.get("user2").is_ok());
}

#[test]
fn test_multiple_inserts_different_keys_allowed() {
    let (mut engine, _dir) = create_test_engine();
    
    let tx_manager = engine.transaction_manager().unwrap();
    let tx_id = tx_manager.begin(IsolationLevel::ReadCommitted).unwrap();
    
    {
        let tx_arc = tx_manager.get(tx_id).unwrap();
        let mut tx = tx_arc.write().unwrap();
        let wal = tx_manager.wal();
        
        let collection = engine.get_collection("_system:users").unwrap();
        
        // Multiple inserts with different keys should be fine
        collection.insert_tx(&mut tx, wal, serde_json::json!({"_key": "user1", "name": "Alice"})).unwrap();
        collection.insert_tx(&mut tx, wal, serde_json::json!({"_key": "user2", "name": "Bob"})).unwrap();
        collection.insert_tx(&mut tx, wal, serde_json::json!({"_key": "user3", "name": "Charlie"})).unwrap();
    }
    
    // Should commit successfully
    engine.commit_transaction(tx_id).unwrap();
    
    let collection = engine.get_collection("_system:users").unwrap();
    assert_eq!(collection.count(), 3);
}
