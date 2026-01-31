use solidb_client::{HttpClient, SoliDBClientBuilder};
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), solidb_client::DriverError> {
    let http_url = "http://127.0.0.1:6745";
    let tcp_addr = "127.0.0.1:6745";
    let iterations = 1000;

    println!("========================================");
    println!("SoliDB Rust Client Benchmark");
    println!("========================================");

    // Test HTTP Client
    println!("\n--- HTTP CLIENT BENCHMARK ---");

    let mut http_client = HttpClient::new(http_url);
    http_client.login("_system", "admin", "admin").await?;
    http_client.create_database("bench_db").await.ok();
    http_client.set_database("bench_db");
    http_client.create_collection("bench_collection").await.ok();

    println!("Inserting {} documents...", iterations);
    let start = Instant::now();
    for i in 0..iterations {
        let doc = serde_json::json!({
            "id": i,
            "data": format!("benchmark data {}", i),
            "timestamp": chrono::Utc::now().timestamp_millis()
        });
        http_client
            .insert("bench_collection", doc, Some(&format!("http_{}", i)))
            .await?;
    }
    let duration = start.elapsed();
    let ops = iterations as f64 / duration.as_secs_f64();
    println!("HTTP INSERT: {:.2} ops/sec", ops);

    println!("Reading {} documents...", iterations);
    let start = Instant::now();
    for i in 0..iterations {
        let _ = http_client
            .get("bench_collection", &format!("http_{}", i))
            .await?;
    }
    let duration = start.elapsed();
    let ops = iterations as f64 / duration.as_secs_f64();
    println!("HTTP READ: {:.2} ops/sec", ops);

    // Test TCP Client
    println!("\n--- TCP CLIENT BENCHMARK ---");

    let mut tcp_client = SoliDBClientBuilder::new(tcp_addr)
        .auth("_system", "admin", "admin")
        .build()
        .await?;

    println!("Inserting {} documents...", iterations);
    let start = Instant::now();
    for i in 0..iterations {
        let doc = serde_json::json!({
            "id": i,
            "data": format!("benchmark data {}", i),
            "timestamp": chrono::Utc::now().timestamp_millis()
        });
        tcp_client
            .insert(
                "bench_db",
                "bench_collection",
                Some(&format!("tcp_{}", i)),
                doc,
            )
            .await?;
    }
    let duration = start.elapsed();
    let ops = iterations as f64 / duration.as_secs_f64();
    println!("TCP INSERT: {:.2} ops/sec", ops);

    println!("Reading {} documents...", iterations);
    let start = Instant::now();
    for i in 0..iterations {
        let _ = tcp_client
            .get("bench_db", "bench_collection", &format!("tcp_{}", i))
            .await?;
    }
    let duration = start.elapsed();
    let ops = iterations as f64 / duration.as_secs_f64();
    println!("TCP READ: {:.2} ops/sec", ops);

    println!("\n========================================");
    println!("Benchmark complete!");
    println!("========================================");

    Ok(())
}
