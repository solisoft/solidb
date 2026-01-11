//! Cursor and Pagination Tests
//!
//! Tests for:
//! - Cursor creation and batching
//! - Fetching multiple batches
//! - Cursor expiration
//! - Parallel cursors

use serde_json::json;
use solidb::server::cursor_store::CursorStore;
use solidb::storage::StorageEngine;
use solidb::{parse, QueryExecutor};
use std::time::Duration;
use tempfile::TempDir;

fn create_test_env() -> (StorageEngine, CursorStore, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");

    // Create large collection
    engine
        .create_collection("numbers".to_string(), None)
        .unwrap();
    let col = engine.get_collection("numbers").unwrap();

    for i in 0..100 {
        col.insert(json!({"_key": format!("n{}", i), "value": i}))
            .unwrap();
    }

    let cursor_store = CursorStore::new(Duration::from_secs(60));

    (engine, cursor_store, tmp_dir)
}

#[tokio::test]
async fn test_cursor_batching() {
    let (engine, cursor_store, _tmp) = create_test_env();

    let query = parse("FOR n IN numbers SORT n.value ASC RETURN n.value").unwrap();
    let executor = QueryExecutor::new(&engine);
    let all_results = executor.execute(&query).unwrap();

    assert_eq!(all_results.len(), 100);

    // Create cursor with batch size 10
    // store() is synchronous
    let cursor_id = cursor_store.store(all_results, 10);

    // Get first batch
    // get_next_batch returns Option<(Vec<Value>, bool)>
    let (batch1, has_more1) = cursor_store
        .get_next_batch(&cursor_id)
        .expect("Should find cursor");
    assert_eq!(batch1.len(), 10);
    assert_eq!(has_more1, true);
    assert_eq!(batch1[0], json!(0));
    assert_eq!(batch1[9], json!(9));

    // Get second batch
    let (batch2, has_more2) = cursor_store
        .get_next_batch(&cursor_id)
        .expect("Should find cursor");
    assert_eq!(batch2.len(), 10);
    assert_eq!(has_more2, true);
    assert_eq!(batch2[0], json!(10));
}

#[tokio::test]
async fn test_cursor_exhaustion() {
    let (engine, cursor_store, _tmp) = create_test_env();

    #[allow(deprecated)]
    // SORT n.value implicit order might vary if not inserted sequentially, but here it is
    let query =
        parse("FOR n IN numbers FILTER n.value < 25 SORT n.value ASC RETURN n.value").unwrap();
    let executor = QueryExecutor::new(&engine);
    let results = executor.execute(&query).unwrap(); // 25 items (0-24)

    // Batch size 10
    let cursor_id = cursor_store.store(results, 10);

    // Batch 1 (0-9)
    let (_b1, more1) = cursor_store.get_next_batch(&cursor_id).unwrap();
    assert!(more1);

    // Batch 2 (10-19)
    let (_b2, more2) = cursor_store.get_next_batch(&cursor_id).unwrap();
    assert!(more2);

    // Batch 3 (20-24) - Last batch
    let (b3, more3) = cursor_store.get_next_batch(&cursor_id).unwrap();
    assert_eq!(b3.len(), 5);
    assert_eq!(more3, false);

    // Try getting next batch after exhaustion - should return None
    let res = cursor_store.get_next_batch(&cursor_id);
    assert!(
        res.is_none(),
        "Should return None when cursor is exhausted/deleted"
    );
}

#[tokio::test]
async fn test_cursor_deletion() {
    let (engine, cursor_store, _tmp) = create_test_env();

    let query = parse("FOR n IN numbers RETURN n").unwrap();
    let executor = QueryExecutor::new(&engine);
    let results = executor.execute(&query).unwrap();

    let cursor_id = cursor_store.store(results, 10);

    // Delete cursor
    let deleted = cursor_store.delete(&cursor_id);
    assert!(deleted);

    // Try to access
    let res = cursor_store.get_next_batch(&cursor_id);
    assert!(res.is_none());
}

#[tokio::test]
async fn test_empty_result_cursor() {
    let (engine, cursor_store, _tmp) = create_test_env();

    let query = parse("FOR n IN numbers FILTER n.value > 1000 RETURN n").unwrap();
    let executor = QueryExecutor::new(&engine);
    let results = executor.execute(&query).unwrap(); // Empty

    let cursor_id = cursor_store.store(results, 10);

    // Getting batch from empty results
    let (batch, has_more) = cursor_store.get_next_batch(&cursor_id).unwrap();
    assert_eq!(batch.len(), 0);
    assert_eq!(has_more, false);

    // Should be deleted now
    assert!(cursor_store.get_next_batch(&cursor_id).is_none());
}

#[tokio::test]
async fn test_large_batch_size() {
    let (engine, cursor_store, _tmp) = create_test_env();

    let query = parse("FOR n IN numbers RETURN n").unwrap();
    let executor = QueryExecutor::new(&engine);
    let results = executor.execute(&query).unwrap(); // 100 items

    // Batch size larger than total results
    let cursor_id = cursor_store.store(results, 200);

    let (batch, has_more) = cursor_store.get_next_batch(&cursor_id).unwrap();
    assert_eq!(batch.len(), 100);
    assert_eq!(has_more, false);
}
