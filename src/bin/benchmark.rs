use serde_json::json;
use solidb_client::{HttpClient, SoliDBClient, SoliDBClientBuilder};
use std::env;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let http_port = env::var("SOLIDB_PORT").unwrap_or_else(|_| "6745".to_string());
    let password = env::var("SOLIDB_PASSWORD").unwrap_or_else(|_| "admin".to_string());

    let http_url = format!("http://127.0.0.1:{}", http_port);
    let tcp_addr = format!("127.0.0.1:6745");

    let iterations = 1000;

    println!("========================================");
    println!("SoliDB Rust Client Benchmark (v0.7.0)");
    println!("========================================");

    // HTTP Client Benchmark
    println!("\n--- HTTP CLIENT BENCHMARK ---");

    let mut http_client = HttpClient::new(&http_url);
    http_client.login("_system", "admin", &password).await?;
    http_client.create_database("bench_db").await.ok();
    http_client.set_database("bench_db");
    http_client.create_collection("rust_bench").await.ok();

    let start = Instant::now();
    for i in 0..iterations {
        let doc = json!({
            "id": i,
            "data": "benchmark data content"
        });
        let key = format!("bench_{}", i);
        http_client.insert("rust_bench", doc, Some(&key)).await?;
    }
    let insert_duration = start.elapsed();
    let insert_ops_per_sec = iterations as f64 / insert_duration.as_secs_f64();
    println!("HTTP INSERT: {:.2} ops/sec", insert_ops_per_sec);

    let start = Instant::now();
    for i in 0..iterations {
        let key = format!("bench_{}", i);
        let _ = http_client.get("rust_bench", &key).await?;
    }
    let read_duration = start.elapsed();
    let read_ops_per_sec = iterations as f64 / read_duration.as_secs_f64();
    println!("HTTP READ: {:.2} ops/sec", read_ops_per_sec);

    // TCP Client Benchmark
    println!("\n--- TCP CLIENT BENCHMARK ---");

    let mut tcp_client = SoliDBClientBuilder::new(&tcp_addr)
        .auth("_system", "admin", &password)
        .build()
        .await?;

    let start = Instant::now();
    for i in 0..iterations {
        let doc = json!({
            "id": i,
            "data": "benchmark data content"
        });
        let key = format!("tcp_{}", i);
        tcp_client
            .insert("bench_db", "rust_bench", Some(&key), doc)
            .await?;
    }
    let insert_duration = start.elapsed();
    let insert_ops_per_sec = iterations as f64 / insert_duration.as_secs_f64();
    println!("TCP INSERT: {:.2} ops/sec", insert_ops_per_sec);

    let start = Instant::now();
    for i in 0..iterations {
        let key = format!("tcp_{}", i);
        let _ = tcp_client.get("bench_db", "rust_bench", &key).await?;
    }
    let read_duration = start.elapsed();
    let read_ops_per_sec = iterations as f64 / read_duration.as_secs_f64();
    println!("TCP READ: {:.2} ops/sec", read_ops_per_sec);

    println!("\n========================================");
    println!("Benchmark complete!");
    println!("========================================");

    Ok(())
}
