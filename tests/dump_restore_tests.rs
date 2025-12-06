use std::process::Command;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_dump_and_restore_collection() {
    // Setup: Start a test server
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    
    // Start server in background
    let mut server = Command::new("cargo")
        .args(&["run", "--bin", "solidb", "--", "--data", db_path.to_str().unwrap(), "--port", "3001"])
        .spawn()
        .expect("Failed to start server");

    // Wait for server to start
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Create test database and collection using HTTP API
    let client = reqwest::blocking::Client::new();
    
    // Create database
    client.post("http://localhost:3001/database")
        .json(&serde_json::json!({ "name": "testdb" }))
        .send()
        .unwrap();

    // Create collection
    client.post("http://localhost:3001/database/testdb/collection")
        .json(&serde_json::json!({ "name": "users" }))
        .send()
        .unwrap();

    // Insert test documents
    for i in 1..=10 {
        client.post("http://localhost:3001/database/testdb/document/users")
            .json(&serde_json::json!({
                "name": format!("User {}", i),
                "age": 20 + i
            }))
            .send()
            .unwrap();
    }

    // Test 1: Dump collection
    let dump_file = temp_dir.path().join("users_dump.json");
    let dump_output = Command::new("cargo")
        .args(&[
            "run", "--bin", "solidb-dump", "--",
            "-d", "testdb",
            "-c", "users",
            "-o", dump_file.to_str().unwrap(),
            "-H", "localhost",
            "-P", "3001",
            "--pretty"
        ])
        .output()
        .expect("Failed to run solidb-dump");

    assert!(dump_output.status.success(), "Dump failed: {:?}", String::from_utf8_lossy(&dump_output.stderr));
    assert!(dump_file.exists(), "Dump file not created");

    // Verify dump contents
    let dump_content = fs::read_to_string(&dump_file).unwrap();
    let dump_json: serde_json::Value = serde_json::from_str(&dump_content).unwrap();
    
    assert_eq!(dump_json["database"], "testdb");
    assert_eq!(dump_json["collections"][0]["name"], "users");
    assert_eq!(dump_json["collections"][0]["documents"].as_array().unwrap().len(), 10);

    // Test 2: Delete collection and restore
    client.delete("http://localhost:3001/database/testdb/collection/users")
        .send()
        .unwrap();

    let restore_output = Command::new("cargo")
        .args(&[
            "run", "--bin", "solidb-restore", "--",
            "-i", dump_file.to_str().unwrap(),
            "-H", "localhost",
            "-P", "3001"
        ])
        .output()
        .expect("Failed to run solidb-restore");

    assert!(restore_output.status.success(), "Restore failed: {:?}", String::from_utf8_lossy(&restore_output.stderr));

    // Verify restored data
    let response = client.get("http://localhost:3001/database/testdb/collection")
        .send()
        .unwrap()
        .json::<serde_json::Value>()
        .unwrap();

    let collections = response["collections"].as_array().unwrap();
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0]["name"], "users");
    assert_eq!(collections[0]["count"], 10);

    // Cleanup
    server.kill().unwrap();
}

#[test]
fn test_dump_entire_database() {
    // Setup
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    
    let mut server = Command::new("cargo")
        .args(&["run", "--bin", "solidb", "--", "--data", db_path.to_str().unwrap(), "--port", "3002"])
        .spawn()
        .expect("Failed to start server");

    std::thread::sleep(std::time::Duration::from_secs(2));

    let client = reqwest::blocking::Client::new();
    
    // Create database
    client.post("http://localhost:3002/database")
        .json(&serde_json::json!({ "name": "multidb" }))
        .send()
        .unwrap();

    // Create multiple collections
    client.post("http://localhost:3002/database/multidb/collection")
        .json(&serde_json::json!({ "name": "users" }))
        .send()
        .unwrap();

    client.post("http://localhost:3002/database/multidb/collection")
        .json(&serde_json::json!({ "name": "posts" }))
        .send()
        .unwrap();

    // Insert data
    client.post("http://localhost:3002/database/multidb/document/users")
        .json(&serde_json::json!({ "name": "Alice" }))
        .send()
        .unwrap();

    client.post("http://localhost:3002/database/multidb/document/posts")
        .json(&serde_json::json!({ "title": "Hello World" }))
        .send()
        .unwrap();

    // Dump entire database
    let dump_file = temp_dir.path().join("full_dump.json");
    let dump_output = Command::new("cargo")
        .args(&[
            "run", "--bin", "solidb-dump", "--",
            "-d", "multidb",
            "-o", dump_file.to_str().unwrap(),
            "-H", "localhost",
            "-P", "3002"
        ])
        .output()
        .expect("Failed to run solidb-dump");

    assert!(dump_output.status.success());

    let dump_content = fs::read_to_string(&dump_file).unwrap();
    let dump_json: serde_json::Value = serde_json::from_str(&dump_content).unwrap();
    
    assert_eq!(dump_json["collections"].as_array().unwrap().len(), 2);

    // Cleanup
    server.kill().unwrap();
}

#[test]
fn test_dump_sharded_collection() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_db");
    
    let mut server = Command::new("cargo")
        .args(&["run", "--bin", "solidb", "--", "--data", db_path.to_str().unwrap(), "--port", "3003"])
        .spawn()
        .expect("Failed to start server");

    std::thread::sleep(std::time::Duration::from_secs(2));

    let client = reqwest::blocking::Client::new();
    
    // Create database
    client.post("http://localhost:3003/database")
        .json(&serde_json::json!({ "name": "sharddb" }))
        .send()
        .unwrap();

    // Create sharded collection
    client.post("http://localhost:3003/database/sharddb/collection")
        .json(&serde_json::json!({
            "name": "sharded_users",
            "numShards": 4,
            "replicationFactor": 2,
            "shardKey": "_key"
        }))
        .send()
        .unwrap();

    // Insert data
    client.post("http://localhost:3003/database/sharddb/document/sharded_users")
        .json(&serde_json::json!({ "name": "Bob" }))
        .send()
        .unwrap();

    // Dump
    let dump_file = temp_dir.path().join("sharded_dump.json");
    let dump_output = Command::new("cargo")
        .args(&[
            "run", "--bin", "solidb-dump", "--",
            "-d", "sharddb",
            "-c", "sharded_users",
            "-o", dump_file.to_str().unwrap(),
            "-H", "localhost",
            "-P", "3003",
            "--pretty"
        ])
        .output()
        .expect("Failed to run solidb-dump");

    assert!(dump_output.status.success());

    // Verify shard config is preserved
    let dump_content = fs::read_to_string(&dump_file).unwrap();
    let dump_json: serde_json::Value = serde_json::from_str(&dump_content).unwrap();
    
    let shard_config = &dump_json["collections"][0]["shardConfig"];
    assert_eq!(shard_config["num_shards"], 4);
    assert_eq!(shard_config["replication_factor"], 2);
    assert_eq!(shard_config["shard_key"], "_key");

    // Cleanup
    server.kill().unwrap();
}
