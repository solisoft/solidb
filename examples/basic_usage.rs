use rust_db::StorageEngine;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== Rust-DB Basic Usage Example ===\n");

    // Initialize storage engine
    let storage = StorageEngine::new("./example_data")?;
    println!("✓ Storage engine initialized");

    // Create a collection
    storage.create_collection("users".to_string())?;
    println!("✓ Created 'users' collection");

    // Insert some documents
    let collection = storage.get_collection("users")?;

    let alice = collection.insert(json!({
        "name": "Alice",
        "age": 30,
        "email": "alice@example.com",
        "active": true
    }))?;
    println!("✓ Inserted document: {}", alice.key);

    let bob = collection.insert(json!({
        "name": "Bob",
        "age": 25,
        "email": "bob@example.com",
        "active": true
    }))?;
    println!("✓ Inserted document: {}", bob.key);

    let charlie = collection.insert(json!({
        "name": "Charlie",
        "age": 35,
        "email": "charlie@example.com",
        "active": false
    }))?;
    println!("✓ Inserted document: {}", charlie.key);

    // Save to disk
    storage.save_collection("users")?;
    println!("✓ Saved collection to disk\n");

    // Execute some AQL queries
    use rust_db::{parse, QueryExecutor};

    let executor = QueryExecutor::new(&storage);

    println!("--- Query 1: Get all users ---");
    let query1 = parse("FOR doc IN users RETURN doc")?;
    let results1 = executor.execute(&query1)?;
    println!("Found {} documents:", results1.len());
    for result in &results1 {
        println!("  {}", serde_json::to_string_pretty(result)?);
    }

    println!("\n--- Query 2: Filter by age > 25 ---");
    let query2 = parse("FOR doc IN users FILTER doc.age > 25 RETURN doc")?;
    let results2 = executor.execute(&query2)?;
    println!("Found {} documents:", results2.len());
    for result in &results2 {
        println!("  {}", serde_json::to_string_pretty(result)?);
    }

    println!("\n--- Query 3: Active users sorted by age ---");
    let query3 = parse("FOR doc IN users FILTER doc.active == true SORT doc.age DESC RETURN doc")?;
    let results3 = executor.execute(&query3)?;
    println!("Found {} documents:", results3.len());
    for result in &results3 {
        println!("  {}", serde_json::to_string_pretty(result)?);
    }

    println!("\n--- Query 4: Project specific fields ---");
    let query4 = parse("FOR doc IN users RETURN {name: doc.name, age: doc.age}")?;
    let results4 = executor.execute(&query4)?;
    println!("Found {} documents:", results4.len());
    for result in &results4 {
        println!("  {}", serde_json::to_string_pretty(result)?);
    }

    println!("\n--- Query 5: Limit results ---");
    let query5 = parse("FOR doc IN users SORT doc.age ASC LIMIT 2 RETURN doc")?;
    let results5 = executor.execute(&query5)?;
    println!("Found {} documents:", results5.len());
    for result in &results5 {
        println!("  {}", serde_json::to_string_pretty(result)?);
    }

    println!("\n✓ Example completed successfully!");

    Ok(())
}
