//! HTTP API Benchmark Suite
//! Tests the performance of the REST API endpoints
//!
//! Usage:
//!   1. Start the server: cargo run --release
//!   2. Run this benchmark: cargo run --release --bin http-benchmark

use rayon::prelude::*;
use reqwest::blocking::Client;
use serde_json::json;
use std::time::{Duration, Instant};

const SERVER_URL: &str = "http://localhost:6745";
const DATABASE: &str = "_system";

// Benchmark sizes
const SMALL: usize = 1_000;
const MEDIUM: usize = 10_000;

// Concurrent benchmark settings
const CONCURRENT_REQUESTS: usize = 100;
const NUM_THREADS: usize = 8;

fn main() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║          SoliDB HTTP API Benchmark Suite                    ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    // Create client with connection pooling and keep-alive
    let client = Client::builder()
        .pool_max_idle_per_host(10) // Keep up to 10 idle connections per host
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .tcp_keepalive(std::time::Duration::from_secs(60))
        .build()
        .expect("Failed to create HTTP client");

    // Check server is running
    match client.get(format!("{}/_api/databases", SERVER_URL)).send() {
        Ok(_) => println!("✅ Server is running at {}\n", SERVER_URL),
        Err(_) => {
            eprintln!("❌ Error: Server is not running at {}", SERVER_URL);
            eprintln!("   Please start the server with: cargo run --release");
            std::process::exit(1);
        }
    }

    // Setup: Create test collection
    setup_collection(&client);

    // Run sequential benchmarks
    bench_insert(&client);
    bench_get_document(&client);
    bench_update_document(&client);
    bench_aql_queries(&client);
    bench_explain_query(&client);
    bench_delete_document(&client);

    // Run concurrent benchmarks
    bench_concurrent();

    // Cleanup
    cleanup(&client);

    println!("\n✅ All HTTP API benchmarks completed!");
}

fn setup_collection(client: &Client) {
    println!("🔧 Setting up test collection...");

    // Delete collection if exists
    let _ = client
        .delete(format!(
            "{}/api/database/{}/collection/bench_http",
            SERVER_URL, DATABASE
        ))
        .send();

    // Create collection
    client
        .post(format!(
            "{}/api/database/{}/collection",
            SERVER_URL, DATABASE
        ))
        .json(&json!({"name": "bench_http"}))
        .send()
        .expect("Failed to create collection");

    println!("   Collection 'bench_http' created\n");
}

fn cleanup(client: &Client) {
    println!("\n🧹 Cleaning up...");
    client
        .delete(format!(
            "{}/api/database/{}/collection/bench_http",
            SERVER_URL, DATABASE
        ))
        .send()
        .expect("Failed to delete collection");
    println!("   Test collection deleted");
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
        "  {:.<45} {:>10} | {:>12} | {} reqs",
        name,
        format_duration(duration),
        format_ops_per_sec(count, duration),
        count
    );
}

fn print_separator() {
    println!("{}", "-".repeat(75));
}

fn bench_insert(client: &Client) {
    println!("📝 INSERT DOCUMENT BENCHMARKS");
    print_separator();

    let url = format!(
        "{}/api/database/{}/document/bench_http",
        SERVER_URL, DATABASE
    );

    // Small batch
    let start = Instant::now();
    for i in 0..SMALL {
        let doc = json!({
            "_key": format!("user_{}", i),
            "name": format!("User {}", i),
            "email": format!("user{}@example.com", i),
            "age": i % 100,
            "active": i % 2 == 0,
            "score": (i * 17) % 1000
        });

        client.post(&url).json(&doc).send().expect("Insert failed");
    }
    print_result("POST /document (small)", SMALL, start.elapsed());

    // Medium batch
    let start = Instant::now();
    for i in SMALL..SMALL + MEDIUM {
        let doc = json!({
            "_key": format!("user_{}", i),
            "name": format!("User {}", i),
            "email": format!("user{}@example.com", i),
            "age": i % 100,
            "active": i % 2 == 0,
            "score": (i * 17) % 1000
        });

        client.post(&url).json(&doc).send().expect("Insert failed");
    }
    print_result("POST /document (medium)", MEDIUM, start.elapsed());

    println!();
}

fn bench_get_document(client: &Client) {
    println!("📖 GET DOCUMENT BENCHMARKS");
    print_separator();

    // Sequential reads
    let start = Instant::now();
    for i in 0..SMALL {
        let url = format!(
            "{}/api/database/{}/document/bench_http/user_{}",
            SERVER_URL, DATABASE, i
        );
        client.get(&url).send().expect("Get failed");
    }
    print_result("GET /document/:key (sequential)", SMALL, start.elapsed());

    // Random reads
    let start = Instant::now();
    for i in 0..SMALL {
        let key_idx = (i * 7919) % (SMALL + MEDIUM); // Prime for pseudo-random
        let url = format!(
            "{}/api/database/{}/document/bench_http/user_{}",
            SERVER_URL, DATABASE, key_idx
        );
        client.get(&url).send().expect("Get failed");
    }
    print_result("GET /document/:key (random)", SMALL, start.elapsed());

    println!();
}

fn bench_update_document(client: &Client) {
    println!("✏️  UPDATE DOCUMENT BENCHMARKS");
    print_separator();

    // Update single field
    let start = Instant::now();
    for i in 0..SMALL {
        let url = format!(
            "{}/api/database/{}/document/bench_http/user_{}",
            SERVER_URL, DATABASE, i
        );
        client
            .put(&url)
            .json(&json!({"score": i * 2}))
            .send()
            .expect("Update failed");
    }
    print_result("PUT /document/:key (single field)", SMALL, start.elapsed());

    // Update multiple fields
    let start = Instant::now();
    for i in 0..SMALL {
        let url = format!(
            "{}/api/database/{}/document/bench_http/user_{}",
            SERVER_URL, DATABASE, i
        );
        client
            .put(&url)
            .json(&json!({
                "score": i * 3,
                "active": false,
                "updated": true,
                "timestamp": chrono::Utc::now().timestamp()
            }))
            .send()
            .expect("Update failed");
    }
    print_result("PUT /document/:key (multi field)", SMALL, start.elapsed());

    println!();
}

fn bench_aql_queries(client: &Client) {
    println!("🔎 AQL QUERY BENCHMARKS");
    print_separator();

    let url = format!("{}/api/database/{}/cursor", SERVER_URL, DATABASE);

    // Simple FOR RETURN
    let query = json!({"query": "FOR u IN bench_http LIMIT 100 RETURN u"});
    let start = Instant::now();
    for _ in 0..SMALL {
        client.post(&url).json(&query).send().expect("Query failed");
    }
    print_result("FOR...LIMIT 100", SMALL, start.elapsed());

    // FOR with FILTER
    let query = json!({"query": "FOR u IN bench_http FILTER u.age > 50 LIMIT 100 RETURN u"});
    let start = Instant::now();
    for _ in 0..SMALL {
        client.post(&url).json(&query).send().expect("Query failed");
    }
    print_result("FOR...FILTER...LIMIT 100", SMALL, start.elapsed());

    // FOR with multiple filters
    let query = json!({"query": "FOR u IN bench_http FILTER u.age > 50 AND u.active == true LIMIT 100 RETURN u"});
    let start = Instant::now();
    for _ in 0..SMALL {
        client.post(&url).json(&query).send().expect("Query failed");
    }
    print_result("FOR...FILTER(AND)...LIMIT 100", SMALL, start.elapsed());

    // SORT query
    let query = json!({"query": "FOR u IN bench_http SORT u.score DESC LIMIT 10 RETURN u"});
    let start = Instant::now();
    for _ in 0..SMALL {
        client.post(&url).json(&query).send().expect("Query failed");
    }
    print_result("SORT...LIMIT 10", SMALL, start.elapsed());

    // Projection
    let query = json!({"query": "FOR u IN bench_http LIMIT 100 RETURN {name: u.name, age: u.age}"});
    let start = Instant::now();
    for _ in 0..SMALL {
        client.post(&url).json(&query).send().expect("Query failed");
    }
    print_result("Projection (100 docs)", SMALL, start.elapsed());

    // COUNT
    let query = json!({"query": "RETURN COLLECTION_COUNT(\"bench_http\")"});
    let start = Instant::now();
    for _ in 0..SMALL {
        client.post(&url).json(&query).send().expect("Query failed");
    }
    print_result("COLLECTION_COUNT", SMALL, start.elapsed());

    // Bind variables
    let query = json!({
        "query": "FOR u IN bench_http FILTER u.age > @minAge LIMIT @limit RETURN u",
        "bindVars": {"minAge": 30, "limit": 50}
    });
    let start = Instant::now();
    for _ in 0..SMALL {
        client.post(&url).json(&query).send().expect("Query failed");
    }
    print_result("Query with bind variables", SMALL, start.elapsed());

    println!();
}

fn bench_explain_query(client: &Client) {
    println!("📊 EXPLAIN QUERY BENCHMARKS");
    print_separator();

    let url = format!("{}/api/database/{}/explain", SERVER_URL, DATABASE);

    // Simple query
    let query = json!({"query": "FOR u IN bench_http LIMIT 100 RETURN u"});
    let start = Instant::now();
    for _ in 0..SMALL {
        client
            .post(&url)
            .json(&query)
            .send()
            .expect("Explain failed");
    }
    print_result("EXPLAIN simple query", SMALL, start.elapsed());

    // Complex query
    let query = json!({"query": "FOR u IN bench_http FILTER u.age > 50 AND u.active == true SORT u.score DESC LIMIT 10 RETURN u"});
    let start = Instant::now();
    for _ in 0..SMALL {
        client
            .post(&url)
            .json(&query)
            .send()
            .expect("Explain failed");
    }
    print_result("EXPLAIN complex query", SMALL, start.elapsed());

    println!();
}

fn bench_delete_document(client: &Client) {
    println!("🗑️  DELETE DOCUMENT BENCHMARKS");
    print_separator();

    let start = Instant::now();
    for i in 0..SMALL {
        let url = format!(
            "{}/api/database/{}/document/bench_http/user_{}",
            SERVER_URL, DATABASE, i
        );
        client.delete(&url).send().expect("Delete failed");
    }
    print_result("DELETE /document/:key", SMALL, start.elapsed());

    println!();
}

fn bench_concurrent() {
    println!("⚡ CONCURRENT BENCHMARKS (Multi-threaded)");
    print_separator();
    println!(
        "  Using {} threads for {} concurrent requests\n",
        NUM_THREADS, CONCURRENT_REQUESTS
    );

    // Configure rayon thread pool
    rayon::ThreadPoolBuilder::new()
        .num_threads(NUM_THREADS)
        .build_global()
        .unwrap();

    // Concurrent GET requests
    let start = Instant::now();
    (0..CONCURRENT_REQUESTS).into_par_iter().for_each(|i| {
        let client = Client::new();
        let key_idx = i % (SMALL + MEDIUM);
        let url = format!(
            "{}/api/database/{}/document/bench_http/user_{}",
            SERVER_URL, DATABASE, key_idx
        );
        client.get(&url).send().expect("Concurrent GET failed");
    });
    print_result(
        "GET /document (concurrent)",
        CONCURRENT_REQUESTS,
        start.elapsed(),
    );

    // Concurrent AQL queries
    let start = Instant::now();
    (0..CONCURRENT_REQUESTS).into_par_iter().for_each(|_| {
        let client = Client::new();
        let url = format!("{}/api/database/{}/cursor", SERVER_URL, DATABASE);
        let query = json!({"query": "FOR u IN bench_http LIMIT 10 RETURN u"});
        client
            .post(&url)
            .json(&query)
            .send()
            .expect("Concurrent query failed");
    });
    print_result(
        "AQL query (concurrent)",
        CONCURRENT_REQUESTS,
        start.elapsed(),
    );

    // Concurrent filtered queries
    let start = Instant::now();
    (0..CONCURRENT_REQUESTS).into_par_iter().for_each(|i| {
        let client = Client::new();
        let url = format!("{}/api/database/{}/cursor", SERVER_URL, DATABASE);
        let min_age = (i % 80) + 20; // Vary the filter
        let query = json!({
            "query": "FOR u IN bench_http FILTER u.age > @minAge LIMIT 10 RETURN u",
            "bindVars": {"minAge": min_age}
        });
        client
            .post(&url)
            .json(&query)
            .send()
            .expect("Concurrent filtered query failed");
    });
    print_result(
        "Filtered query (concurrent)",
        CONCURRENT_REQUESTS,
        start.elapsed(),
    );

    // Concurrent COUNT queries
    let start = Instant::now();
    (0..CONCURRENT_REQUESTS).into_par_iter().for_each(|_| {
        let client = Client::new();
        let url = format!("{}/api/database/{}/cursor", SERVER_URL, DATABASE);
        let query = json!({"query": "RETURN COLLECTION_COUNT(\"bench_http\")"});
        client
            .post(&url)
            .json(&query)
            .send()
            .expect("Concurrent COUNT failed");
    });
    print_result(
        "COLLECTION_COUNT (concurrent)",
        CONCURRENT_REQUESTS,
        start.elapsed(),
    );

    println!();
}
