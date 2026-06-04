//! Temporary perf probe: list_collections (in-memory cf_names vs disk list_cf)
//! and per-CF create cost as CF count grows.
//!
//! Run: cargo run --release --example cf_perf_probe

use solidb::storage::{RocksDb, StorageEngine};
use std::time::Instant;

fn main() {
    let dir = std::env::temp_dir().join(format!("cf_probe_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let engine = StorageEngine::new(&dir).expect("open engine");
    engine.initialize().expect("init");
    let db = engine.get_database("_system").expect("get db");

    // --- CF create scaling ---
    println!("--- create_collection cost as CF count grows ---");
    for batch in 0..10 {
        let start = Instant::now();
        for i in 0..100 {
            db.create_collection(format!("coll_{}_{}", batch, i), None)
                .expect("create");
        }
        let total_cfs = (batch + 1) * 100;
        println!(
            "create #{:4}..{:4}: {:>8.1?} total, {:>7.2?}/collection",
            batch * 100,
            total_cfs,
            start.elapsed(),
            start.elapsed() / 100
        );
    }

    // --- list_collections: new (in-memory) vs old (disk MANIFEST read) ---
    println!("\n--- list_collections with ~1000 CFs, 200 iterations ---");

    let start = Instant::now();
    let mut n = 0;
    for _ in 0..200 {
        n = engine.list_collections().len();
    }
    println!(
        "new  (cf_names, in-memory): {:>9.2?} total, {:>9.2?}/call  ({} collections)",
        start.elapsed(),
        start.elapsed() / 200,
        n
    );

    let start = Instant::now();
    let mut n = 0;
    for _ in 0..200 {
        n = RocksDb::list_cf(&rust_rocksdb::Options::default(), &dir)
            .unwrap_or_default()
            .len();
    }
    println!(
        "old  (list_cf, disk read) : {:>9.2?} total, {:>9.2?}/call  ({} CFs)",
        start.elapsed(),
        start.elapsed() / 200,
        n
    );

    // --- point reads (bloom filter + shared cache apply to new SSTs) ---
    println!("\n--- 10k point gets on a 10k-doc collection ---");
    let coll = db.get_or_create_collection("bench_docs").expect("coll");
    for i in 0..10_000 {
        let doc = serde_json::json!({"_key": format!("k{}", i), "v": i});
        coll.insert(doc).expect("insert");
    }
    engine.flush().expect("flush");

    let start = Instant::now();
    for i in 0..10_000 {
        let _ = coll.get(&format!("k{}", i)).expect("get");
    }
    println!(
        "10k gets: {:>9.2?} total, {:>9.2?}/get",
        start.elapsed(),
        start.elapsed() / 10_000
    );

    drop(coll);
    drop(db);
    drop(engine);
    let _ = std::fs::remove_dir_all(&dir);
}
