use serde_json::json;
use solidb::driver::protocol::IsolationLevel;
use solidb::driver::SoliDBClient;
use std::env;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = env::var("SOLIDB_PORT").unwrap_or_else(|_| "9998".to_string());
    let password = env::var("SOLIDB_PASSWORD").unwrap_or_else(|_| "password".to_string());
    let addr = format!("127.0.0.1:{}", port);

    let db = "bench_tx_db";
    let col = "tx_bench";

    // Setup
    let mut client = SoliDBClient::connect(&addr).await?;
    if let Err(_) = client.auth("_system", "admin", &password).await {
        println!("Auth warning: check server logs if this fails.");
    }

    // Clean start
    let _ = client.delete_database(db).await;
    client.create_database(db).await?;
    client.create_collection(db, col, None).await?;

    let iterations = 1000;

    println!("Starting Transaction Benchmarks ({} ops)...", iterations);

    // 1. BASELINE: Local Auto-Commit (No explicit TX)
    // Each op is technically atomic, but no multi-op TX overhead
    let start = Instant::now();
    for i in 0..iterations {
        let doc = json!({ "id": i, "val": "baseline" });
        client
            .insert(db, col, Some(&format!("base_{}", i)), doc)
            .await?;
    }
    let duration = start.elapsed();
    let baseline_ops = iterations as f64 / duration.as_secs_f64();
    println!("Baseline (No TX): {:.2} ops/sec", baseline_ops);

    // 2. BULK TRANSACTION: One huge active transaction
    // Should mitigate WAL fsyncs if the server groups them (depends on implementation)
    // But locking overhead exists.
    let _ = client.delete_collection(db, col).await;
    client.create_collection(db, col, None).await?;

    let start = Instant::now();
    client
        .begin_transaction(db, Some(IsolationLevel::ReadCommitted))
        .await?;
    for i in 0..iterations {
        let doc = json!({ "id": i, "val": "bulk" });
        client
            .insert(db, col, Some(&format!("bulk_{}", i)), doc)
            .await?;
    }
    client.commit().await?;
    let duration = start.elapsed();
    let bulk_ops = iterations as f64 / duration.as_secs_f64();
    println!(
        "Bulk TX (1 TX, {} ops): {:.2} ops/sec",
        iterations, bulk_ops
    );

    // 3. MANY TRANSACTIONS: High overhead
    let _ = client.delete_collection(db, col).await;
    client.create_collection(db, col, None).await?;

    let start = Instant::now();
    for i in 0..iterations {
        client
            .begin_transaction(db, Some(IsolationLevel::ReadCommitted))
            .await?;
        let doc = json!({ "id": i, "val": "many" });
        client
            .insert(db, col, Some(&format!("many_{}", i)), doc)
            .await?;
        client.commit().await?;
    }
    let duration = start.elapsed();
    let many_ops = iterations as f64 / duration.as_secs_f64();
    println!(
        "Many TXs ({} TXs, 1 op each): {:.2} ops/sec",
        iterations, many_ops
    );

    // Cleanup
    let _ = client.delete_database(db).await;

    println!("\nSummary:");
    if bulk_ops > baseline_ops {
        println!(
            "  - Bulk Transactions IMPROVED performance by {:.1}%",
            (bulk_ops - baseline_ops) / baseline_ops * 100.0
        );
    } else {
        println!(
            "  - Bulk Transactions DECREASED performance by {:.1}%",
            (baseline_ops - bulk_ops) / baseline_ops * 100.0
        );
    }

    println!(
        "  - Individual Transactions overhead: {:.1}% slowdown",
        (baseline_ops - many_ops) / baseline_ops * 100.0
    );

    Ok(())
}
