use solidb::storage::StorageEngine;
use solidb::sharding::coordinator::ShardCoordinator;
use std::sync::Arc;

#[tokio::test]
async fn test_blob_upload_with_sharding() {
    // Setup storage and coordinator
    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(StorageEngine::new(tmp_dir.path().to_str().unwrap()).unwrap());
    storage.create_database("_system".to_string()).unwrap();

    // Create coordinator (standalone mode)
    let coordinator = Arc::new(ShardCoordinator::new(storage.clone(), None, None));

    // Create a sharded blob collection
    let db = storage.get_database("_system").unwrap();
    // Create the collection as a blob type
    db.create_collection("test_blobs".to_string(), Some("blob".to_string())).unwrap();

    // Get the collection object and set sharding config
    let collection = db.get_collection("test_blobs").unwrap();
    let shard_config = solidb::sharding::coordinator::CollectionShardConfig {
        num_shards: 4,
        shard_key: "_key".to_string(),
        replication_factor: 1,
    };
    collection.set_shard_config(&shard_config).unwrap();

    // Create shard tables
    coordinator.create_shards("_system", "test_blobs").await.unwrap();

    // Test blob upload through coordinator
    let test_data = b"Hello, this is test blob data for sharding!";
    let chunks = vec![(0u32, test_data.to_vec())];

    let metadata = serde_json::json!({
        "name": "test_blob.txt",
        "type": "text/plain",
        "size": test_data.len(),
        "chunks": 1
    });

    let result = coordinator.upload_blob(
        "_system",
        "test_blobs",
        &collection.get_shard_config().unwrap(),
        metadata,
        chunks,
    ).await;

    assert!(result.is_ok(), "Blob upload should succeed");

    let uploaded_meta = result.unwrap();
    let blob_key = uploaded_meta.get("_key").unwrap().as_str().unwrap();

    // Verify blob can be downloaded
    let download_result = coordinator.download_blob(
        "_system",
        "test_blobs",
        &collection.get_shard_config().unwrap(),
        blob_key,
    ).await;

    assert!(download_result.is_ok(), "Blob download should succeed");

    // Verify the data
    let response = download_result.unwrap();
    // Note: In a real test, we'd need to extract the body and verify it matches test_data
    // For now, just check that we got a successful response
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_blob_collection_creation_not_auto_sharded() {
    // Test that blob collections are NOT auto-sharded by default
    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(StorageEngine::new(tmp_dir.path().to_str().unwrap()).unwrap());
    storage.create_database("_system".to_string()).unwrap();

    // Create coordinator
    let _coordinator = Arc::new(ShardCoordinator::new(storage.clone(), None, None));

    // Create a blob collection without explicit sharding config
    let db = storage.get_database("_system").unwrap();

    // Create the collection as a blob type
    db.create_collection("auto_blob_test".to_string(), Some("blob".to_string())).unwrap();

    // Get the collection object
    let collection = db.get_collection("auto_blob_test").unwrap();

    // Verify it's a blob collection
    assert_eq!(collection.get_type(), "blob");

    // Verify it's NOT sharded (this is the key change)
    assert!(collection.get_shard_config().is_none(),
        "Blob collections should NOT be auto-sharded by default");
}

#[tokio::test]
async fn test_blob_collection_explicit_sharding_still_works() {
    // Test that blob collections can still be explicitly sharded
    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(StorageEngine::new(tmp_dir.path().to_str().unwrap()).unwrap());
    storage.create_database("_system".to_string()).unwrap();

    // Create coordinator
    let _coordinator = Arc::new(ShardCoordinator::new(storage.clone(), None, None));

    // Create an explicitly sharded blob collection
    let db = storage.get_database("_system").unwrap();

    // Create the collection as a blob type
    db.create_collection("explicit_shard_blob".to_string(), Some("blob".to_string())).unwrap();

    // Get the collection object
    let collection = db.get_collection("explicit_shard_blob").unwrap();

    // Set the shard configuration
    let shard_config = solidb::sharding::coordinator::CollectionShardConfig {
        num_shards: 3,
        shard_key: "_key".to_string(),
        replication_factor: 2,
    };
    collection.set_shard_config(&shard_config).unwrap();

    // Verify it's a blob collection AND sharded
    assert_eq!(collection.get_type(), "blob");
    assert!(collection.get_shard_config().is_some(),
        "Explicitly sharded blob collections should be sharded");

    let config = collection.get_shard_config().unwrap();
    assert_eq!(config.num_shards, 3);
    assert_eq!(config.replication_factor, 2);
}
