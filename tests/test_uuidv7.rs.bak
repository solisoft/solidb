use solidb::StorageEngine;
use tempfile::TempDir;
use std::thread;
use std::time::Duration;

#[test]
fn test_uuidv7_time_ordering() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    
    // Create collection
    storage.create_collection("test".to_string()).unwrap();
    let collection = storage.get_collection("test").unwrap();
    
    // Insert documents with small time gaps
    let doc1 = collection.insert(serde_json::json!({"value": 1})).unwrap();
    thread::sleep(Duration::from_millis(10));
    
    let doc2 = collection.insert(serde_json::json!({"value": 2})).unwrap();
    thread::sleep(Duration::from_millis(10));
    
    let doc3 = collection.insert(serde_json::json!({"value": 3})).unwrap();
    
    // Extract keys
    let key1 = &doc1.key;
    let key2 = &doc2.key;
    let key3 = &doc3.key;
    
    // UUIDv7 should be lexicographically sortable by time
    assert!(key1 < key2, "UUIDv7 key1 should be less than key2: {} < {}", key1, key2);
    assert!(key2 < key3, "UUIDv7 key2 should be less than key3: {} < {}", key2, key3);
    
    println!("✓ UUIDv7 keys are time-ordered:");
    println!("  doc1: {}", key1);
    println!("  doc2: {}", key2);
    println!("  doc3: {}", key3);
}
