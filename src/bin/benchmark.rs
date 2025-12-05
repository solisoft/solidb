//! Database Benchmark Suite
//! Run with: cargo run --release --bin benchmark

use serde_json::json;
use solidb::{parse, IndexType, QueryExecutor, StorageEngine};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const SMALL_DATASET: usize = 500;
const MEDIUM_DATASET: usize = 5_000;
const LARGE_DATASET: usize = 50_000;

fn main() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║              SoliDB Benchmark Suite                          ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");

    // Run benchmarks
    bench_insert(&storage);
    bench_read_by_key(&storage);
    bench_update(&storage);
    bench_delete(&storage);
    bench_index_lookup(&storage);
    bench_aql_queries(&storage);
    bench_range_queries(&storage);

    println!("\n✅ All benchmarks completed!");
}

fn format_duration(d: Duration) -> String {
    if d.as_secs() > 0 {
        format!("{:.2}s", d.as_secs_f64())
    } else if d.as_millis() > 0 {
        format!("{:.2}ms", d.as_secs_f64() * 1000.0)
    } else {
        format!("{:.2}µs", d.as_secs_f64() * 1_000_000.0)
    }
}

fn format_ops_per_sec(count: usize, d: Duration) -> String {
    let ops = count as f64 / d.as_secs_f64();
    if ops >= 1_000_000.0 {
        format!("{:.2}M ops/s", ops / 1_000_000.0)
    } else if ops >= 1_000.0 {
        format!("{:.2}K ops/s", ops / 1_000.0)
    } else {
        format!("{:.2} ops/s", ops)
    }
}

fn print_result(name: &str, count: usize, duration: Duration) {
    println!(
        "  {:.<40} {:>10} | {:>12} | {} docs",
        name,
        format_duration(duration),
        format_ops_per_sec(count, duration),
        count
    );
}

fn print_separator() {
    println!("{}", "-".repeat(70));
}

fn get_city(i: usize) -> &'static str {
    const CITIES: [&str; 5] = ["Paris", "London", "Berlin", "Tokyo", "NYC"];
    CITIES[i % 5]
}

fn bench_insert(storage: &StorageEngine) {
    println!("📝 INSERT BENCHMARKS");
    print_separator();

    // Small dataset
    storage
        .create_collection("bench_insert_small".to_string())
        .unwrap();
    let collection = storage.get_collection("bench_insert_small").unwrap();

    let start = Instant::now();
    for i in 0..SMALL_DATASET {
        collection
            .insert(json!({
                "name": format!("User{}", i),
                "email": format!("user{}@example.com", i),
                "age": i % 100,
                "city": get_city(i),
                "score": (i * 17) % 1000
            }))
            .unwrap();
    }
    print_result("Insert (small)", SMALL_DATASET, start.elapsed());

    // Medium dataset
    storage
        .create_collection("bench_insert_medium".to_string())
        .unwrap();
    let collection = storage.get_collection("bench_insert_medium").unwrap();

    let start = Instant::now();
    for i in 0..MEDIUM_DATASET {
        collection
            .insert(json!({
                "name": format!("User{}", i),
                "email": format!("user{}@example.com", i),
                "age": i % 100,
                "city": get_city(i),
                "score": (i * 17) % 1000
            }))
            .unwrap();
    }
    print_result("Insert (medium)", MEDIUM_DATASET, start.elapsed());

    // Large dataset with custom keys
    storage
        .create_collection("bench_users".to_string())
        .unwrap();
    let collection = storage.get_collection("bench_users").unwrap();

    let start = Instant::now();
    for i in 0..LARGE_DATASET {
        collection
            .insert(json!({
                "_key": format!("user_{}", i),
                "name": format!("User{}", i),
                "email": format!("user{}@example.com", i),
                "age": i % 100,
                "city": get_city(i),
                "active": i % 2 == 0,
                "score": (i * 17) % 1000
            }))
            .unwrap();
    }
    print_result("Insert (large, with keys)", LARGE_DATASET, start.elapsed());
    println!();
}

fn bench_read_by_key(storage: &StorageEngine) {
    println!("📖 READ BY KEY BENCHMARKS");
    print_separator();

    let collection = storage.get_collection("bench_users").unwrap();

    // Sequential reads
    let start = Instant::now();
    for i in 0..MEDIUM_DATASET {
        let _ = collection.get(&format!("user_{}", i));
    }
    print_result("Read by key (sequential)", MEDIUM_DATASET, start.elapsed());

    // Random reads
    let start = Instant::now();
    for i in 0..MEDIUM_DATASET {
        let key = format!("user_{}", (i * 7919) % LARGE_DATASET); // Prime for pseudo-random
        let _ = collection.get(&key);
    }
    print_result("Read by key (random)", MEDIUM_DATASET, start.elapsed());
    println!();
}

fn bench_update(storage: &StorageEngine) {
    println!("✏️  UPDATE BENCHMARKS");
    print_separator();

    let collection = storage.get_collection("bench_users").unwrap();

    // Update single field
    let start = Instant::now();
    for i in 0..SMALL_DATASET {
        let key = format!("user_{}", i);
        collection.update(&key, json!({"score": i * 2})).unwrap();
    }
    print_result("Update single field", SMALL_DATASET, start.elapsed());

    // Update multiple fields
    let start = Instant::now();
    for i in 0..SMALL_DATASET {
        let key = format!("user_{}", i);
        collection
            .update(
                &key,
                json!({
                    "score": i * 3,
                    "active": false,
                    "updated": true
                }),
            )
            .unwrap();
    }
    print_result("Update multiple fields", SMALL_DATASET, start.elapsed());
    println!();
}

fn bench_delete(storage: &StorageEngine) {
    println!("🗑️  DELETE BENCHMARKS");
    print_separator();

    // Create a collection for delete tests
    storage
        .create_collection("bench_delete".to_string())
        .unwrap();
    let collection = storage.get_collection("bench_delete").unwrap();

    // Insert documents
    for i in 0..SMALL_DATASET {
        collection
            .insert(json!({
                "_key": format!("del_{}", i),
                "name": format!("DeleteMe{}", i)
            }))
            .unwrap();
    }

    let start = Instant::now();
    for i in 0..SMALL_DATASET {
        collection.delete(&format!("del_{}", i)).unwrap();
    }
    print_result("Delete by key", SMALL_DATASET, start.elapsed());
    println!();
}

fn bench_index_lookup(storage: &StorageEngine) {
    println!("🔍 INDEX BENCHMARKS");
    print_separator();

    let collection = storage.get_collection("bench_users").unwrap();

    // Create index on age field
    let start = Instant::now();
    collection
        .create_index(
            "idx_age".to_string(),
            "age".to_string(),
            IndexType::Persistent,
            false,
        )
        .unwrap();
    println!(
        "  Index creation (age, {}K docs)........ {:>10}",
        LARGE_DATASET / 1000,
        format_duration(start.elapsed())
    );

    // Create index on city field
    let start = Instant::now();
    collection
        .create_index(
            "idx_city".to_string(),
            "city".to_string(),
            IndexType::Hash,
            false,
        )
        .unwrap();
    println!(
        "  Index creation (city, {}K docs)....... {:>10}",
        LARGE_DATASET / 1000,
        format_duration(start.elapsed())
    );

    // Index lookups
    let start = Instant::now();
    for i in 0..SMALL_DATASET {
        let _ = collection.index_lookup_eq("age", &json!(i % 100));
    }
    print_result("Index lookup (age)", SMALL_DATASET, start.elapsed());

    let cities = ["Paris", "London", "Berlin", "Tokyo", "NYC"];
    let start = Instant::now();
    for i in 0..SMALL_DATASET {
        let _ = collection.index_lookup_eq("city", &json!(cities[i % 5]));
    }
    print_result(
        "Index lookup (city, ~10K each)",
        SMALL_DATASET,
        start.elapsed(),
    );

    // City lookup with limit (more realistic)
    let start = Instant::now();
    for i in 0..SMALL_DATASET {
        let _ = collection.index_lookup_eq_limit("city", &json!(cities[i % 5]), 100);
    }
    print_result(
        "Index lookup (city, LIMIT 100)",
        SMALL_DATASET,
        start.elapsed(),
    );
    println!();
}

fn bench_aql_queries(storage: &StorageEngine) {
    println!("🔎 AQL QUERY BENCHMARKS");
    print_separator();

    // Simple FOR RETURN
    let query = parse("FOR u IN bench_users LIMIT 100 RETURN u").unwrap();
    let start = Instant::now();
    for _ in 0..100 {
        let executor = QueryExecutor::new(storage);
        let _ = executor.execute(&query);
    }
    print_result("FOR...LIMIT 100 (x100 runs)", 100, start.elapsed());

    // FOR with FILTER
    let query =
        parse(r#"FOR u IN bench_users FILTER u.city == "Paris" LIMIT 100 RETURN u"#).unwrap();
    let start = Instant::now();
    for _ in 0..100 {
        let executor = QueryExecutor::new(storage);
        let _ = executor.execute(&query);
    }
    print_result("FOR...FILTER...LIMIT 100 (x100)", 100, start.elapsed());

    // FOR with multiple filters
    let query =
        parse(r#"FOR u IN bench_users FILTER u.city == "Paris" AND u.age > 50 LIMIT 100 RETURN u"#)
            .unwrap();
    let start = Instant::now();
    for _ in 0..100 {
        let executor = QueryExecutor::new(storage);
        let _ = executor.execute(&query);
    }
    print_result("FOR...FILTER(AND)...LIMIT (x100)", 100, start.elapsed());

    // Projection
    let query =
        parse(r#"FOR u IN bench_users LIMIT 1000 RETURN { name: u.name, city: u.city }"#).unwrap();
    let start = Instant::now();
    for _ in 0..10 {
        let executor = QueryExecutor::new(storage);
        let _ = executor.execute(&query);
    }
    print_result("Projection (1K docs, x10)", 10, start.elapsed());

    // COUNT
    let query = parse("RETURN COLLECTION_COUNT(\"bench_users\")").unwrap();
    let start = Instant::now();
    for _ in 0..SMALL_DATASET {
        let executor = QueryExecutor::new(storage);
        let _ = executor.execute(&query);
    }
    print_result("COLLECTION_COUNT", SMALL_DATASET, start.elapsed());

    // SORT
    let query = parse("FOR u IN bench_users SORT u.score DESC LIMIT 10 RETURN u").unwrap();
    let start = Instant::now();
    for _ in 0..10 {
        let executor = QueryExecutor::new(storage);
        let _ = executor.execute(&query);
    }
    print_result("SORT...LIMIT 10 (x10 runs)", 10, start.elapsed());
    println!();
}

fn bench_range_queries(storage: &StorageEngine) {
    println!("📊 RANGE & COMPUTATION BENCHMARKS");
    print_separator();

    // Range generation
    let query = parse("RETURN 1..1000").unwrap();
    let start = Instant::now();
    for _ in 0..SMALL_DATASET {
        let executor = QueryExecutor::new(storage);
        let _ = executor.execute(&query);
    }
    print_result("Range 1..1000", SMALL_DATASET, start.elapsed());

    // Range with FOR
    let query = parse("FOR i IN 1..100 RETURN i * 2").unwrap();
    let start = Instant::now();
    for _ in 0..SMALL_DATASET {
        let executor = QueryExecutor::new(storage);
        let _ = executor.execute(&query);
    }
    print_result("FOR i IN 1..100 RETURN i*2", SMALL_DATASET, start.elapsed());

    // String functions
    let query = parse(r#"FOR i IN 1..100 RETURN CONCAT("User", i)"#).unwrap();
    let start = Instant::now();
    for _ in 0..100 {
        let executor = QueryExecutor::new(storage);
        let _ = executor.execute(&query);
    }
    print_result("CONCAT in loop (x100)", 100, start.elapsed());

    // Date functions
    let query = parse(r#"RETURN DATE_NOW()"#).unwrap();
    let start = Instant::now();
    for _ in 0..MEDIUM_DATASET {
        let executor = QueryExecutor::new(storage);
        let _ = executor.execute(&query);
    }
    print_result("DATE_NOW()", MEDIUM_DATASET, start.elapsed());

    // Math functions
    let query = parse("FOR i IN 1..100 RETURN SQRT(i)").unwrap();
    let start = Instant::now();
    for _ in 0..SMALL_DATASET {
        let executor = QueryExecutor::new(storage);
        let _ = executor.execute(&query);
    }
    print_result("SQRT in loop", SMALL_DATASET, start.elapsed());
    println!();
}
