use std::process::{Command, Stdio};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn test_port_multiplexing() {
    // 1. Setup
    let port = 9511; // Use unique port
    let data_dir = "/tmp/test_multiplex_db_9511";
    let _ = std::fs::remove_dir_all(data_dir);
    
    println!("Starting server on port {}", port);
    
    // Use the binary directly since we built it
    // Stderr/Stdout inherited to avoid pipe deadlock if we don't read them
    let mut child = Command::new("./target/debug/solidb")
        .args(&[
            "--port", &port.to_string(), 
            "--replication-port", &port.to_string(), 
            "--data-dir", data_dir,
            "--node-id", "test_node_mux"
        ])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Failed to start solidb binary. Did you run 'cargo build'?");

    // 3. Verify HTTP Access
    println!("Waiting for server at http://127.0.0.1:{}/_api/health...", port);
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/_api/health", port);
    println!("Waiting for server at {}...", url);
    
    let start = std::time::Instant::now();
    let mut ready = false;
    
    while start.elapsed() < Duration::from_secs(15) {
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                ready = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    
    if !ready {
        let _ = child.kill();
        panic!("Server failed to respond to HTTP health check within 15s");
    }
    
    println!("Server is ready!");

    // 3. Test HTTP Access (Redundant but confirms)
    println!("Testing HTTP access...");
    println!("Testing HTTP access...");
    let client = reqwest::Client::new();
    let resp = client.get(format!("http://127.0.0.1:{}/_api/health", port))
        .send()
        .await;
        
    match resp {
        Ok(r) => {
            assert!(r.status().is_success(), "HTTP Health check failed: status {}", r.status());
            println!("HTTP Access: OK");
        },
        Err(e) => {
            let _ = child.kill();
            panic!("HTTP Connection failed: {}", e);
        }
    }

    // 4. Test Sync Access (Protocol Handshake)
    println!("Testing Sync access...");
    match TcpStream::connect(format!("127.0.0.1:{}", port)).await {
        Ok(mut stream) => {
            // A. Send Magic Header
            if let Err(e) = stream.write_all(b"solidb-sync-v1").await {
                let _ = child.kill();
                panic!("Failed to write magic header: {}", e);
            }
            
            // B. Read response length
            let mut header = [0u8; 5];
            if let Err(e) = stream.read_exact(&mut header).await {
                let _ = child.kill();
                panic!("Failed to read Main response header: {}", e);
            }
            
            let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);
            println!("Received response length: {}", len);
            
            let mut payload = vec![0u8; len as usize];
            if let Err(e) = stream.read_exact(&mut payload).await {
                let _ = child.kill();
                panic!("Failed to read payload: {}", e);
            }
            
            println!("Sync Access: OK (Received {} bytes)", len);
        },
        Err(e) => {
            let _ = child.kill();
            panic!("Failed to connect to TCP port: {}", e);
        }
    }

    // Cleanup
    let _ = child.kill();
    let _ = std::fs::remove_dir_all(data_dir);
}

