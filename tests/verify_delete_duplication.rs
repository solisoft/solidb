use solidb::storage::engine::StorageEngine;
use solidb::storage::collection::{ChangeType, ChangeEvent};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn test_delete_event_duplication() {
    let temp_dir = TempDir::new().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    storage.initialize().unwrap();

    // Create a sharded collection
    storage.create_collection("test_collection".to_string(), None).unwrap();
    let collection = storage.get_collection("test_collection").unwrap();

    // Subscribe to changes
    let mut rx = collection.change_sender.subscribe();

    // Insert a document
    let doc_json = serde_json::json!({"name": "Test", "value": 123});
    let inserted_doc = collection.insert(doc_json).unwrap();

    // Wait for insert event
    let insert_event = rx.recv().await.unwrap();
    assert!(matches!(insert_event.type_, ChangeType::Insert));
    assert_eq!(insert_event.key, inserted_doc.key);

    // Delete the document
    collection.delete(&inserted_doc.key).unwrap();

    // Check how many DELETE events we receive
    let mut delete_count = 0;

    // Wait a bit for events to arrive
    sleep(Duration::from_millis(50)).await;

    // Try to receive events for a short time
    while let Ok(event) = tokio::time::timeout(Duration::from_millis(10), rx.recv()).await {
        match event {
            Ok(event) => {
                if matches!(event.type_, ChangeType::Delete) && event.key == inserted_doc.key {
                    delete_count += 1;
                    println!("Received DELETE event #{}", delete_count);
                }
            }
            Err(_) => break,
        }
    }

    println!("Total DELETE events received: {}", delete_count);
    assert_eq!(delete_count, 1, "Expected exactly 1 DELETE event, but got {}", delete_count);
}
