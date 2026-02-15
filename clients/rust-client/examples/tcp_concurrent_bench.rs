use solidb_client::SoliDBClientBuilder;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), solidb_client::DriverError> {
    let tcp_addr = "127.0.0.1:6745";
    let total_ops = 100_000; // More ops for higher concurrency
    let concurrency = 200;

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║       SoliDB TCP Client Concurrent Benchmark              ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    // Create TCP client
    let mut tcp_client = SoliDBClientBuilder::new(tcp_addr)
        .auth("_system", "admin", "admin")
        .build()
        .await?;

    // Create test collection
    tcp_client.create_collection("_system", "tcp_bench200", None).await.ok();

    let ops_per_worker = total_ops / concurrency;
    println!("--- TCP CONCURRENT BENCHMARK ---\n");
    println!("Total operations: {}", total_ops);
    println!("Concurrency: {} parallel workers\n", concurrency);
    println!("Ops per worker: {}\n", ops_per_worker);

    // Concurrent INSERT benchmark with 200 workers
    println!("📝 TCP INSERT ({} concurrent workers)...", concurrency);
    let start = Instant::now();
    
    let mut handles = vec![];
    for worker_id in 0..concurrency {
        let handle = tokio::spawn(async move {
            let mut client = SoliDBClientBuilder::new(tcp_addr)
                .auth("_system", "admin", "admin")
                .build()
                .await
                .unwrap();
            
            for i in 0..ops_per_worker {
                let doc = serde_json::json!({
                    "id": i,
                    "worker": worker_id,
                    "data": format!("benchmark data {}-{}", worker_id, i),
                });
                client
                    .insert(
                        "_system",
                        "tcp_bench200",
                        Some(&format!("w{}_{}", worker_id, i)),
                        doc,
                    )
                    .await
                    .unwrap();
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }
    
    let duration = start.elapsed();
    let ops = total_ops as f64 / duration.as_secs_f64();
    println!("   TCP INSERT: {:.2} ops/sec ({} total in {:.2?})", ops, total_ops, duration);

    // Concurrent READ benchmark with 200 workers
    println!("\n📖 TCP READ ({} concurrent workers)...", concurrency);
    let start = Instant::now();
    
    let mut handles = vec![];
    for worker_id in 0..concurrency {
        let handle = tokio::spawn(async move {
            let mut client = SoliDBClientBuilder::new(tcp_addr)
                .auth("_system", "admin", "admin")
                .build()
                .await
                .unwrap();
            
            for i in 0..ops_per_worker {
                let _ = client
                    .get("_system", "tcp_bench200", &format!("w{}_{}", worker_id, i))
                    .await
                    .unwrap();
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }
    
    let duration = start.elapsed();
    let ops = total_ops as f64 / duration.as_secs_f64();
    println!("   TCP READ: {:.2} ops/sec ({} total in {:.2?})", ops, total_ops, duration);

    println!("\n✅ Benchmark complete!");
    
    Ok(())
}
