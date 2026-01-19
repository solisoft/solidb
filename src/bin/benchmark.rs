use serde_json::json;
use solidb_client::SoliDBClient;
use std::env;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = env::var("SOLIDB_PORT").unwrap_or_else(|_| "9998".to_string());
    let password = env::var("SOLIDB_PASSWORD").unwrap_or_else(|_| "password".to_string());

    let addr = format!("127.0.0.1:{}", port);
    let mut client = SoliDBClient::connect(&addr).await?;
    client.auth("_system", "admin", &password).await?;

    let db = "bench_db";
    let col = "rust_bench";

    let _ = client.create_database(db).await;
    let _ = client.create_collection(db, col, None).await;

    let iterations = 1000;

    // INSERT BENCHMARK
    let start = Instant::now();
    for i in 0..iterations {
        let doc = json!({
            "id": i,
            "data": "benchmark data content"
        });
        let key = format!("bench_{}", i);
        client.insert(db, col, Some(&key), doc).await?;
    }
    let insert_duration = start.elapsed();
    let insert_ops_per_sec = iterations as f64 / insert_duration.as_secs_f64();
    println!("RUST_BENCH_RESULT:{:.2}", insert_ops_per_sec);

    // READ BENCHMARK
    let start = Instant::now();
    for i in 0..iterations {
        let key = format!("bench_{}", i);
        let _ = client.get(db, col, &key).await?;
    }
    let read_duration = start.elapsed();
    let read_ops_per_sec = iterations as f64 / read_duration.as_secs_f64();
    println!("RUST_READ_BENCH_RESULT:{:.2}", read_ops_per_sec);

    Ok(())
}
