use serde_json::json;
use solidb::driver::SoliDBClient;
use std::env;
use std::time::Instant;
use tokio::task::JoinSet;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = env::var("SOLIDB_PORT").unwrap_or_else(|_| "9998".to_string());
    let password = env::var("SOLIDB_PASSWORD").unwrap_or_else(|_| "password".to_string());

    let num_workers = 16;
    let total_inserts = 10000;
    let inserts_per_worker = total_inserts / num_workers;

    let db = "bench_db";
    let col = "rust_parallel_bench";

    // Setup: create database and collection
    let addr = format!("127.0.0.1:{}", port);
    let mut setup_client = SoliDBClient::connect(&addr).await?;
    setup_client.auth("_system", "admin", &password).await?;
    let _ = setup_client.create_database(db).await;
    let _ = setup_client.create_collection(db, col, None).await;
    drop(setup_client);

    let start = Instant::now();

    let mut set = JoinSet::new();

    for worker_id in 0..num_workers {
        let addr = addr.clone();
        let password = password.clone();

        set.spawn(async move {
            let mut client = SoliDBClient::connect(&addr).await.unwrap();
            client.auth("_system", "admin", &password).await.unwrap();

            for i in 0..inserts_per_worker {
                let doc = json!({
                    "worker": worker_id,
                    "id": i,
                    "data": "parallel benchmark data"
                });
                let _ = client.insert(db, col, None, doc).await;
            }
        });
    }

    // Wait for all workers
    while let Some(_) = set.join_next().await {}

    let duration = start.elapsed();
    let ops_per_sec = total_inserts as f64 / duration.as_secs_f64();

    println!("RUST_PARALLEL_BENCH_RESULT:{:.2}", ops_per_sec);

    Ok(())
}
