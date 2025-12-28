use anyhow::{Result, Context};
use reqwest::Client;
use serde_json::json;
use solidb::{
    cluster::{ClusterConfig, manager::ClusterManager, transport::TcpTransport, state::ClusterState, node::Node},
    storage::StorageEngine,
    sync::{log::SyncLog, worker::{SyncWorker, SyncConfig, create_command_channel}, state::SyncState, transport::ConnectionPool},
    sharding::coordinator::ShardCoordinator,
    server::multiplex::ChannelListener,
    create_router,
};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

struct TestNode {
    pub base_url: String,
    pub repl_address: String,
    pub _dir: TempDir, // Keep alive
    pub port: u16,
}

// Spawns a full solidb node in-process
async fn spawn_node(peers: Vec<String>) -> Result<TestNode> {
    let dir = TempDir::new()?;
    let bind_addr = "127.0.0.1:0";
    let listener = TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    let port = local_addr.port();
    
    let ip = local_addr.ip().to_string();
    let port_str = port.to_string();
    let address = format!("{}:{}", ip, port_str);
    
    // Identity
    let node_id = uuid::Uuid::new_v4().to_string();
    
    // Create Keyfile
    let keyfile_path = dir.path().join("solidb.key");
    std::fs::write(&keyfile_path, "test_secret_key").unwrap();
    let keyfile = keyfile_path.to_str().unwrap().to_string();

    std::env::set_var("SOLIDB_CLUSTER_SECRET", "test_secret");

    // Node
    let node = Node::new(node_id.clone(), address.clone(), address.clone());
    
    // Cluster Config
    let cluster_config = ClusterConfig::new(
        Some(node_id.clone()),
        peers.clone(),
        port,
        None
    );

    // Storage
    let storage = StorageEngine::with_cluster_config(dir.path().to_str().unwrap(), cluster_config.clone())?;
    storage.initialize()?;

    // Create Admin User manually
    if let Ok(db) = storage.get_database("_system") {
        if db.get_collection("users").is_err() {
            let _ = db.create_collection("users".to_string(), None);
        }
        if let Ok(coll) = db.get_collection("users") {
            let hash = solidb::server::auth::AuthService::hash_password("admin").unwrap();
            let _ = coll.upsert_batch(vec![
                ("admin".to_string(), serde_json::json!({
                    "username": "admin",
                    "password_hash": hash,
                    "active": true,
                    "role": "admin",
                    "created_at": chrono::Utc::now().to_rfc3339()
                }))
            ]);
        }
    }

    let storage_arc = Arc::new(storage.clone());

    // Components
    let transport = Arc::new(TcpTransport::new(address.clone()));
    let cluster_state = ClusterState::new(node_id.clone());
    
    let replication_log = Arc::new(SyncLog::new(
        node_id.clone(),
        dir.path().to_str().unwrap(),
        1000
    ).map_err(|e| anyhow::anyhow!(e))?);

    let cluster_manager = Arc::new(ClusterManager::new(
        node.clone(),
        cluster_state,
        transport.clone(),
        Some(replication_log.clone()),
        Some(storage_arc.clone())
    ));

    // Start Manager
    let mgr_clone = cluster_manager.clone();
    tokio::spawn(async move {
        mgr_clone.start().await;
    });

    // Shard Coordinator
    let shard_coordinator = Arc::new(ShardCoordinator::new(
        storage_arc.clone(),
        Some(cluster_manager.clone()),
        Some(replication_log.clone())
    ));
    
    // Healing Task (background)
    let heal_coord = shard_coordinator.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await; // Fast heal for test
            let _ = heal_coord.heal_shards().await;
        }
    });

    // Sync Worker
    let sync_state = Arc::new(SyncState::new(storage_arc.clone(), node_id.clone()));
    let conn_pool = Arc::new(ConnectionPool::new(node_id.clone(), keyfile.clone()));
    let (_tx, worker_cmd_rx) = create_command_channel();
    
    let sync_worker = SyncWorker::new(
        storage_arc.clone(),
        sync_state,
        conn_pool,
        replication_log.clone(),
        SyncConfig::default(),
        worker_cmd_rx,
        node_id.clone(),
        keyfile,
        address.clone()
    )
    .with_cluster_manager(cluster_manager.clone())
    .with_shard_coordinator(shard_coordinator.clone());

    // Router
    let app = create_router(
        storage.clone(),
        Some(cluster_manager.clone()),
        Some(replication_log.clone()),
        Some(shard_coordinator.clone()),
        None,
        Arc::new(solidb::scripting::ScriptStats::default()),
        port
    );

    // Join Seeds if any
    if !peers.is_empty() {
        let mgr_join = cluster_manager.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            for peer in peers {
                let _ = mgr_join.join_cluster(&peer).await;
            }
        });
    }

    // Multiplexing & Serving
    let (http_tx, http_rx) = mpsc::channel(100);
    let (sync_tx, sync_rx) = mpsc::channel(100);
    
    // HTTP Server
    let channel_listener = ChannelListener::new(http_rx, local_addr);
    tokio::spawn(async move {
        axum::serve(channel_listener, app).await.unwrap();
    });

    // Sync Worker Background
    let sync_worker = sync_worker.with_incoming_channel(sync_rx);
    tokio::spawn(async move {
        sync_worker.run_background().await;
    });

    // Dispatch Loop
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        loop {
            let (socket, addr) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => continue,
            };
            
            let mut socket = socket;
            let mut buf = [0u8; 16];
            
            // Peek
            let n = match socket.peek(&mut buf).await {
                Ok(n) => n,
                Err(_) => continue,
            };
            
            if n > 0 {
                // Determine protocol
                let is_http = &buf[..3] == b"GET" || &buf[..4] == b"POST" || &buf[..3] == b"PUT" || &buf[..4] == b"HEAD" || &buf[..6] == b"DELETE";
                // Check prefix for Sync
                let is_sync = n >= 14 && &buf[0..14] == b"solidb-sync-v1";
                let is_json = buf[0] == b'{';
                
                if is_http {
                    let peeked = solidb::server::multiplex::PeekedStream::new(socket, vec![]);
                    let _ = http_tx.send((peeked, addr)).await;
                } else if is_sync {
                    // Sync: Consume the magic header bytes so worker receives stream starting at Frame Length
                    let mut magic = [0u8; 14];
                    let _ = tokio::io::AsyncReadExt::read_exact(&mut socket, &mut magic).await;
                    // Pass RAW socket (wrapped)
                    let _ = sync_tx.send((Box::new(socket), addr.to_string())).await;
                } else if is_json {
                    // Handle Cluster Message (JSON)
                    // Use Peekedstream to keep data
                    let peeked = solidb::server::multiplex::PeekedStream::new(socket, vec![]);
                    let mgr = cluster_manager.clone();
                    tokio::spawn(async move {
                         let mut stream = peeked;
                         let mut buf = Vec::new();
                         if let Ok(_) = tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut buf).await {
                            if let Ok(msg) = serde_json::from_slice(&buf) {
                                mgr.handle_message(msg).await;
                            } else {
                                println!("Failed to deserialize cluster message from {}", addr);
                            }
                         }
                    });
                } else {
                    // Fallback / Garbage
                    // Maybe print error?
                    println!("Unknown protocol on port {}", local_addr.port());
                }
            }
        }
    });

    Ok(TestNode {
        base_url: format!("http://{}", address),
        repl_address: address,
        _dir: dir,
        port
    })
}

#[tokio::test]
async fn verify_sharding_repair() -> Result<()> {
    tracing_subscriber::fmt::init();
    
    println!("Spawning Node 1...");
    let node1 = spawn_node(vec![]).await?;
    println!("Node 1 at {}", node1.base_url);

    println!("Spawning Node 2...");
    let node2 = spawn_node(vec![node1.repl_address.clone()]).await?;
    println!("Node 2 at {}", node2.base_url);

    // Wait for cluster mesh
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Use Node 1 as entry point
    let client = Client::new();
    let base_url = &node1.base_url;

    // Login
    let login_res = client.post(format!("{}/auth/login", base_url))
        .json(&json!({"username": "admin", "password": "admin"}))
        .send().await?;
    
    let token = if login_res.status().is_success() {
        let body: serde_json::Value = login_res.json().await?;
        Some(body["token"].as_str().unwrap().to_string())
    } else {
        println!("Login failed (maybe disabled or not ready). Proceeding without auth.");
        None
    };

    let client = if let Some(t) = token {
        Client::builder()
            .default_headers({
                let mut h = reqwest::header::HeaderMap::new();
                h.insert("Authorization", format!("Bearer {}", t).parse()?);
                h
            })
            .build()?
    } else {
        Client::new()
    };

    // Create DB
    client.post(format!("{}/_api/database", base_url))
        .json(&json!({"name": "test_repair_db"}))
        .send().await?;
    
    // Create Shards Manually (Physical)
    // We create them on Node 1 (s0) and Node 2 (s1) directly?
    // Using `_api/document` on specific nodes?
    // Actually, `repair` assumes physical shards exist.
    // We simulate "Truncate" failure where shards exist but data is misplaced?
    // Or simpler:
    // Create sharded collection normally.
    // Insert data.
    // Manually INSERT misplaced data into physical shards via direct access?
    // Direct access to physical shards is RESTRICTED?
    // No, `_api/collection` allows creation of any name.
    // We can create `testcoll_s0` manually?
    // But `ShardedCollection` creates them.
    
    // Let's create collection with 2 shards.
    let res = client.post(format!("{}/_api/database/test_repair_db/collection", base_url))
        .json(&json!({
            "name": "repair_test",
            "numShards": 2,
            "replicationFactor": 1
        }))
        .send().await?;
    assert!(res.status().is_success(), "Failed to create collection");

    // Start with clean state
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Insert normally (Placed correctly)
    client.post(format!("{}/_api/database/test_repair_db/document/repair_test", base_url))
        .json(&json!({"_key": "doc_correct", "val": 1}))
        .send().await?;
    
    // Now simulate INCONSISTENCY.
    // We want a duplicate of "doc_correct" on the WRONG shard.
    // We need to know where `doc_correct` lives. 
    // We'll insert "doc_duplicate" into BOTH `repair_test_s0` and `repair_test_s1`.
    // One is correct, one is redundant.
    client.post(format!("{}/_api/database/test_repair_db/document/repair_test_s0", base_url))
        .json(&json!({"_key": "doc_duplicate", "val": 2}))
        .send().await?;
    
    client.post(format!("{}/_api/database/test_repair_db/document/repair_test_s1", base_url))
        .json(&json!({"_key": "doc_duplicate", "val": 2}))
        .send().await?;

    // Verify duplication exists (Count should be 2 instead of 1 for that key?)
    
    // Run Repair
    let res = client.post(format!("{}/_api/database/test_repair_db/collection/repair_test/repair", base_url))
        .send().await?;
    
    let report_str = res.text().await?;
    println!("Repair Report: {}", report_str);
    
    // Verify
    assert!(report_str.contains("Deleted 1 duplicates") || report_str.contains("Moved") || report_str.contains("Deleted"), "Report should show action");

    println!("Repair Action Validation Passed...");
    Ok(())
}
