use solidb::driver::SoliDBClient;
use serde_json::json;
use std::time::Instant;
use std::env;

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
    let mut inserted_keys = Vec::new();
    let start = Instant::now();
    for i in 0..iterations {
        let doc = json!({
            "id": i,
            "data": "benchmark data content"
        });
        let result = client.insert(db, col, None, doc).await?;
        if let Some(key) = result.get("_key").and_then(|k| k.as_str()) {
            inserted_keys.push(key.to_string());
        }
    }
    let insert_duration = start.elapsed();
    let insert_ops_per_sec = iterations as f64 / insert_duration.as_secs_f64();
    println!("RUST_BENCH_RESULT:{:.2}", insert_ops_per_sec);

    // READ BENCHMARK
    if !inserted_keys.is_empty() {
        let start = Instant::now();
        for i in 0..iterations {
            let key = &inserted_keys[i % inserted_keys.len()];
            let _ = client.get(db, col, key).await?;
        }
        let read_duration = start.elapsed();
        let read_ops_per_sec = iterations as f64 / read_duration.as_secs_f64();
        println!("RUST_READ_BENCH_RESULT:{:.2}", read_ops_per_sec);
    }

    Ok(())
}
