use solidb::storage::engine::StorageEngine;

use tempfile::TempDir;
use std::time::Instant;

#[tokio::test]
async fn benchmark_websocket_overhead() {
    let temp_dir = TempDir::new().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    storage.initialize().unwrap();

    storage.create_collection("bench_ws".to_string(), None).unwrap();
    let collection = storage.get_collection("bench_ws").unwrap();

    let num_docs = 20_000;
    
    // --- Baseline: No subscribers ---
    println!("Running baseline benchmark (no subscribers)...");
    let start = Instant::now();
    for i in 0..num_docs {
        let doc = serde_json::json!({
            "_key": format!("base_{}", i),
            "value": i,
            "data": "some string data to make payload realistic"
        });
        collection.insert(doc).unwrap();
    }
    let duration_base = start.elapsed();
    let ops_base = num_docs as f64 / duration_base.as_secs_f64();
    println!("Baseline: {:?} ({:.2} ops/sec)", duration_base, ops_base);

    // --- With Subscribers ---
    println!("Running benchmark with subscribers...");
    
    // Create new collection for clean state (avoid RocksDB compaction noise affecting results)
    storage.create_collection("bench_ws_active".to_string(), None).unwrap();
    let collection_active = storage.get_collection("bench_ws_active").unwrap();

    // Spawn subscribers
    let num_subscribers = 50;
    for _ in 0..num_subscribers {
        let mut rx = collection_active.change_sender.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                   Ok(event) => {
                       let _min_cost = serde_json::to_string(&event).unwrap();
                   }
                   Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                   Err(_) => break,
                }
            }
        });
    }

    // Allow time for subscriptions to settle
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let start = Instant::now();
    for i in 0..num_docs {
        let doc = serde_json::json!({
            "_key": format!("active_{}", i),
            "value": i,
            "data": "some string data to make payload realistic"
        });
        collection_active.insert(doc).unwrap();
    }
    let duration_active = start.elapsed();
    let ops_active = num_docs as f64 / duration_active.as_secs_f64();
    println!("With Subscribers: {:?} ({:.2} ops/sec)", duration_active, ops_active);
    
    if ops_active < ops_base {
        let slow_down = (ops_base - ops_active) / ops_base * 100.0;
        println!("Performance Impact: {:.2}% slowdown", slow_down);
    } else {
        println!("Performance Impact: None (or faster due to noise)");
    }
}
