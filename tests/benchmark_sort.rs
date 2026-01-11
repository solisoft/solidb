//! SDBQL Sort Benchmark
//!
//! Benchmarks sorting performance on a collection.

use serde_json::json;
use solidb::sdbql::QueryExecutor;
use solidb::storage::StorageEngine;
use std::time::Instant;
use tempfile::TempDir;

fn create_test_db() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");

    engine.create_database("bench_db".to_string()).unwrap();
    (engine, tmp_dir)
}

#[test]
#[ignore] // Don't run by default as it's slow
fn benchmark_sort_performance() {
    let (engine, _tmp) = create_test_db();
    let db = engine.get_database("bench_db").unwrap();
    db.create_collection("items".to_string(), None).unwrap();
    let items = db.get_collection("items").unwrap();

    let count = 10_000;
    println!("Inserting {} documents...", count);

    // Insert shuffled data
    for i in 0..count {
        // Use a value that is partly random/shuffled
        let val = (i * 17) % count;
        items
            .insert(json!({ "val": val, "padding": "x".repeat(100) }))
            .unwrap();
    }

    let executor = QueryExecutor::with_database(&engine, "bench_db".to_string());

    println!("Executing SORT query...");
    let start = Instant::now();
    let query_ast = solidb::sdbql::parse("FOR i IN items SORT i.val ASC RETURN i.val").unwrap();
    let result = executor.execute(&query_ast).unwrap();
    let duration = start.elapsed();

    println!("Sorted {} items in {:?}", result.len(), duration);
    assert_eq!(result.len(), count as usize);

    // Verify order
    let mut prev = -1;
    for val in result {
        let curr = val.as_i64().unwrap();
        assert!(curr >= prev);
        prev = curr;
    }
}
