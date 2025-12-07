use solidb::storage::engine::StorageEngine;
use solidb::storage::collection::{ChangeType, ChangeEvent};
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn test_changefeed_logic() {
    let temp_dir = TempDir::new().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    storage.initialize().unwrap();

    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();

    // Subscribe to changes
    let mut rx = collection.change_sender.subscribe();

    // 1. Test Insert
    let doc_json = serde_json::json!({"name": "Alice", "age": 30});
    let inserted_doc = collection.insert(doc_json).unwrap();
    
    let event = rx.recv().await.unwrap();
    assert!(matches!(event.type_, ChangeType::Insert));
    assert_eq!(event.key, inserted_doc.key);
    assert_eq!(event.data.unwrap()["name"], "Alice");
    assert!(event.old_data.is_none());

    // 2. Test Update
    let update_json = serde_json::json!({"age": 31});
    collection.update(&inserted_doc.key, update_json).unwrap();

    let event = rx.recv().await.unwrap();
    assert!(matches!(event.type_, ChangeType::Update));
    assert_eq!(event.key, inserted_doc.key);
    assert_eq!(event.data.as_ref().unwrap()["age"], 31);
    assert_eq!(event.old_data.as_ref().unwrap()["age"], 30);

    // 3. Test Delete
    collection.delete(&inserted_doc.key).unwrap();

    let event = rx.recv().await.unwrap();
    assert!(matches!(event.type_, ChangeType::Delete));
    assert_eq!(event.key, inserted_doc.key);
    assert!(event.data.is_none());
    assert_eq!(event.old_data.as_ref().unwrap()["name"], "Alice");
}
