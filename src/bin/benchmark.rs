use solidb::driver::SoliDBClient;
use serde_json::json;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = SoliDBClient::connect("127.0.0.1:9998").await?;
    client.auth("_system", "admin", "bench").await?;

    let db = "bench_db";
    let col = "rust_bench";

    let _ = client.create_database(db).await;
    let _ = client.create_collection(db, col, None).await;

    let iterations = 1000;
    
    let start = Instant::now();
    for i in 0..iterations {
        let doc = json!({
            "id": i,
            "data": "benchmark data content"
        });
        client.insert(db, col, None, doc).await?;
    }
    let duration = start.elapsed();
    
    let ops_per_sec = iterations as f64 / duration.as_secs_f64();
    
    println!("RUST_BENCH_RESULT:{:.2}", ops_per_sec);

    Ok(())
}
