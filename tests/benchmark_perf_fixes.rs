//! Performance benchmarks for the query-engine bottleneck fixes:
//! index-metadata caching, clone-free field access, decorated sort,
//! and LIMIT pushdown into index lookups.
//!
//! Run with:
//!   cargo test --release --test benchmark_perf_fixes -- --ignored --nocapture

use serde_json::json;
use solidb::sdbql::QueryExecutor;
use solidb::storage::{IndexType, StorageEngine};
use std::time::Instant;
use tempfile::TempDir;

fn create_test_db() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine.create_database("bench_db".to_string()).unwrap();
    (engine, tmp_dir)
}

fn seed_items(engine: &StorageEngine, count: usize) {
    let db = engine.get_database("bench_db").unwrap();
    db.create_collection("items".to_string(), None).unwrap();
    let items = db.get_collection("items").unwrap();

    for chunk_start in (0..count).step_by(5000) {
        let chunk_end = (chunk_start + 5000).min(count);
        let batch: Vec<_> = (chunk_start..chunk_end)
            .map(|i| {
                json!({
                    "val": (i * 17) % 1000,
                    "name": format!("item-{}", i),
                    "padding": "x".repeat(100),
                })
            })
            .collect();
        items.insert_batch(batch).unwrap();
    }
}

fn run_query(executor: &QueryExecutor, query: &str, label: &str) -> usize {
    // Warm-up run to populate block cache so we measure CPU, not cold I/O.
    let ast = solidb::sdbql::parse(query).unwrap();
    executor.execute(&ast).unwrap();

    let iterations = 5;
    let start = Instant::now();
    let mut len = 0;
    for _ in 0..iterations {
        let result = executor.execute(&ast).unwrap();
        len = result.len();
    }
    let avg = start.elapsed() / iterations;
    println!("{label}: {len} rows, avg {avg:?} over {iterations} runs");
    len
}

#[test]
#[ignore]
fn benchmark_full_scan_filter() {
    let (engine, _tmp) = create_test_db();
    seed_items(&engine, 100_000);
    let executor = QueryExecutor::with_database(&engine, "bench_db".to_string());

    let len = run_query(
        &executor,
        "FOR d IN items FILTER d.val == 42 RETURN d.name",
        "full-scan FILTER",
    );
    assert_eq!(len, 100);
}

#[test]
#[ignore]
fn benchmark_sort_limit_unindexed() {
    let (engine, _tmp) = create_test_db();
    seed_items(&engine, 100_000);
    let executor = QueryExecutor::with_database(&engine, "bench_db".to_string());

    let len = run_query(
        &executor,
        "FOR d IN items SORT d.val ASC, d.name DESC LIMIT 10 RETURN d.name",
        "SORT+LIMIT (no index)",
    );
    assert_eq!(len, 10);
}

#[test]
#[ignore]
fn benchmark_indexed_filter_limit() {
    let (engine, _tmp) = create_test_db();
    seed_items(&engine, 100_000);
    let db = engine.get_database("bench_db").unwrap();
    let items = db.get_collection("items").unwrap();
    items
        .create_index(
            "val_idx".to_string(),
            vec!["val".to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();
    let executor = QueryExecutor::with_database(&engine, "bench_db".to_string());

    let len = run_query(
        &executor,
        "FOR d IN items FILTER d.val == 42 LIMIT 10 RETURN d.name",
        "indexed FILTER+LIMIT",
    );
    assert_eq!(len, 10);
}

#[test]
#[ignore]
fn benchmark_single_inserts_with_indexes() {
    let (engine, _tmp) = create_test_db();
    let db = engine.get_database("bench_db").unwrap();
    db.create_collection("items".to_string(), None).unwrap();
    let items = db.get_collection("items").unwrap();
    for (name, field) in [("idx_a", "a"), ("idx_b", "b"), ("idx_c", "c")] {
        items
            .create_index(
                name.to_string(),
                vec![field.to_string()],
                IndexType::Persistent,
                false,
            )
            .unwrap();
    }

    let count = 10_000;
    let start = Instant::now();
    for i in 0..count {
        items
            .insert(json!({ "a": i, "b": i % 100, "c": format!("c-{}", i) }))
            .unwrap();
    }
    let elapsed = start.elapsed();
    println!(
        "single inserts with 3 indexes: {count} docs in {elapsed:?} ({:.0} docs/s)",
        count as f64 / elapsed.as_secs_f64()
    );
}
