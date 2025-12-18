use anyhow::Result;
use serde_json::json;
use tokio::time::{sleep, Duration};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use std::process::{Child, Command, Stdio};
use std::path::PathBuf;

// Helper to spawn a solidb process
struct SolidbProcess {
    child: Child,
    port: u16,
    _repl_port: u16,
    data_dir: PathBuf,
}

impl SolidbProcess {
    fn start(port: u16, repl_port: u16, node_id: &str, peers: &[u16]) -> Result<Self> {
        // Create temp dir but persist it so child process can use it
        // and we delete it manually in Drop
        let temp_dir = tempfile::tempdir()?;
        let path = temp_dir.path().to_path_buf();
        // Prevent automatic cleanup by `temp_dir` drop, we own cleanup in SolidbProcess::drop
        let _ = temp_dir.keep();

        let data_dir = path.join(node_id);
        std::fs::create_dir_all(&data_dir)?;

        // Create keyfile for TCP auth
        let keyfile_path = path.join("cluster.key");
        if !keyfile_path.exists() {
            std::fs::write(&keyfile_path, "secret123_tcp_key_must_be_long_enough_for_hmac_sha256_so_lets_make_it_so")?;
        }

        let mut args = vec![
            "run".to_string(),
            "--".to_string(),
            "--port".to_string(), port.to_string(),
            "--replication-port".to_string(), repl_port.to_string(),
            "--data-dir".to_string(), data_dir.to_string_lossy().to_string(),
            "--node-id".to_string(), node_id.to_string(),
            "--keyfile".to_string(), keyfile_path.to_string_lossy().to_string(),
        ];

        for peer_port in peers {
            args.push("--peer".to_string());
            args.push(format!("127.0.0.1:{}", peer_port));
        }

        // Print args for debugging
        println!("Starting node {} with args: {:?}", node_id, args);

        // Set environment variables for auth and secret
        let child = Command::new("cargo")
            .args(&args)
            .env("SOLIDB_ADMIN_PASSWORD", "admin")
            .env("SOLIDB_CLUSTER_SECRET", "secret123") // Restored for HTTP auth
            .env("RUST_LOG", "debug") // DEBUG level
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?;

        Ok(Self {
            child,
            port,
            _repl_port: repl_port,
            data_dir,
        })
    }
}

impl Drop for SolidbProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }
}

#[tokio::test]
async fn test_sharded_changefeed_cluster() -> Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // 1. Build project first
    let status = Command::new("cargo").arg("build").status()?;
    assert!(status.success(), "Failed to build project");

    // 2. Start Node 1 and Node 2
    // Mutual peering: Node 1 knows about Node 2's repl port (6783)
    let node1 = SolidbProcess::start(6780, 6781, "node1", &[6783])?;
    sleep(Duration::from_secs(2)).await;

    // Node 2 knows about Node 1's repl port (6781)
    let node2 = SolidbProcess::start(6782, 6783, "node2", &[6781])?;
    sleep(Duration::from_secs(10)).await; // Increase wait

    let client = reqwest::Client::new();
    let base_url1 = format!("http://127.0.0.1:{}", node1.port);
    let base_url2 = format!("http://127.0.0.1:{}", node2.port);

    // Wait for health check
    wait_for_health(&client, &base_url1).await?;
    wait_for_health(&client, &base_url2).await?;

    // Verify cluster formation
    wait_for_cluster_size(&client, &base_url1, 2).await?;
    wait_for_cluster_size(&client, &base_url2, 2).await?;

    // 3. Create a SHARDED collection
    let create_body = json!({
        "name": "feed_test",
        "numShards": 2,
        "replicationFactor": 1
    });

    // Auth header
    let auth_header = "Basic YWRtaW46YWRtaW4="; // admin:admin

    let res = client.post(format!("{}/_api/database/_system/collection", base_url1))
        .header("Authorization", auth_header)
        .json(&create_body)
        .send()
        .await?;

    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    tracing::info!("Create Collection Response: {} - {}", status, text);

    // Wait for collection to propagate to Node 2
    wait_for_collection(&client, &base_url2, "feed_test").await?;

    // 4. Get User Token for WS
    let login_body = json!({
        "username": "admin",
        "password": "admin"
    });
    let res = client.post(format!("{}/auth/login", base_url1))
        .json(&login_body)
        .send()
        .await?;

    let status = res.status();
    if !status.is_success() {
         let text = res.text().await.unwrap_or_default();
         panic!("Login failed: {} - {}", status, text);
    }

    let body: serde_json::Value = res.json().await?;
    let token = body["token"].as_str().expect("No token returned");


    // 5. Connect WebSocket Changefeed to NODE 1
    let ws_url = format!("ws://127.0.0.1:{}/_api/ws/changefeed?token={}", node1.port, token);

    tracing::info!("Connecting WS to {}", ws_url);
    let (ws_stream, _) = connect_async(ws_url).await?;
    let (mut write, mut read) = ws_stream.split();

    // Subscribe
    write.send(tokio_tungstenite::tungstenite::protocol::Message::Text(json!({
        "type": "subscribe",
        "collection": "feed_test"
    }).to_string().into())).await?;

    // Wait for "subscribed"
    if let Some(msg) = read.next().await {
        let text = msg?.to_text()?.to_string();
        tracing::info!("WS Response: {}", text);
        assert!(text.contains("subscribed"));
    }

    // 6. Perform Operations on NODE 2 (Remote to the WS connection)
    // We expect Node 1 to receive events even if they occur on Node 2's shard

    // INSERT
    let doc_body = json!({
        "foo": "bar",
        "_key": "doc1"
    });
    let res = client.post(format!("{}/_api/database/_system/document/feed_test", base_url2))
        .header("Authorization", auth_header)
        .json(&doc_body)
        .send()
        .await?;

    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    assert_eq!(status, 200, "Insert failed on Node 2: {}", text);

    // Verify INSERT event
    let event_msg = timeout_recv(&mut read).await?;
    let event: serde_json::Value = serde_json::from_str(&event_msg)?;
    tracing::info!("Received Event: {:?}", event);
    assert_eq!(event["type"], "insert");
    assert_eq!(event["key"], "doc1");

    // UPDATE
    let update_body = json!({
        "foo": "baz"
    });
    // Use PUT or PATCH
    let res = client.put(format!("{}/_api/database/_system/document/feed_test/doc1", base_url2))
         .header("Authorization", auth_header)
        .json(&update_body)
        .send()
        .await?;

    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    assert_eq!(status, 200, "Update failed on Node 2: {}", text);

    // Verify UPDATE event
    let event_msg = timeout_recv(&mut read).await?;
    let event: serde_json::Value = serde_json::from_str(&event_msg)?;
    tracing::info!("Received Event: {:?}", event);
    assert_eq!(event["type"], "update");
    assert_eq!(event["key"], "doc1");

    // DELETE
    let res = client.delete(format!("{}/_api/database/_system/document/feed_test/doc1", base_url2))
         .header("Authorization", auth_header)
        .send()
        .await?;
    assert!(res.status().is_success(), "Delete failed on Node 2");

    // Verify DELETE event
    let event_msg = timeout_recv(&mut read).await?;
    let event: serde_json::Value = serde_json::from_str(&event_msg)?;
    tracing::info!("Received Event: {:?}", event);
    assert_eq!(event["type"], "delete");
    assert_eq!(event["key"], "doc1");

    Ok(())
}

async fn wait_for_health(client: &reqwest::Client, base_url: &str) -> Result<()> {
    for _ in 0..60 { // 30 seconds max
        if let Ok(res) = client.get(format!("{}/_api/health", base_url)).send().await {
            if res.status().is_success() {
                return Ok(());
            }
        }
        sleep(Duration::from_millis(500)).await;
    }
    Err(anyhow::anyhow!("Timeout waiting for health at {}", base_url))
}

async fn timeout_recv(read: &mut futures::stream::SplitStream<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>) -> Result<String> {
    match tokio::time::timeout(Duration::from_secs(5), read.next()).await {
        Ok(Some(Ok(msg))) => Ok(msg.to_text()?.to_string()),
        Ok(Some(Err(e))) => Err(anyhow::anyhow!("WS Error: {}", e)),
        Ok(None) => Err(anyhow::anyhow!("WS Closed")),
        Err(_) => Err(anyhow::anyhow!("Timeout waiting for event")),
    }
}

async fn wait_for_cluster_size(client: &reqwest::Client, base_url: &str, size: usize) -> Result<()> {
    let auth = "Basic YWRtaW46YWRtaW4=";
    for _ in 0..60 {
        if let Ok(res) = client.get(format!("{}/_api/cluster/info", base_url))
            .header("Authorization", auth)
            .send().await {
            if res.status().is_success() {
                if let Ok(body) = res.json::<serde_json::Value>().await {
                    // Check for peers in cluster_config (if exists)
                    let peer_count = if let Some(cluster_config) = body.get("cluster_config") {
                        if let Some(peers) = cluster_config.get("peers").and_then(|p| p.as_array()) {
                            peers.len()
                        } else {
                            0
                        }
                    } else {
                        0
                    };

                    tracing::debug!("Cluster info: {:?}, peer_count: {}", body, peer_count);

                    // For a 2-node cluster, each node should see 1 peer
                    if peer_count >= size - 1 {
                        return Ok(());
                    }
                }
                }
            }

        sleep(Duration::from_millis(500)).await;
    }
    Err(anyhow::anyhow!("Timeout waiting for cluster size {} at {}", size, base_url))
}

async fn wait_for_collection(client: &reqwest::Client, base_url: &str, name: &str) -> Result<()> {
    let auth = "Basic YWRtaW46YWRtaW4=";
    for _ in 0..60 {
        if let Ok(res) = client.get(format!("{}/_api/database/_system/collection/{}", base_url, name))
            .header("Authorization", auth)
            .send()
            .await
        {
            if res.status().is_success() {
                return Ok(());
            }
        }
        sleep(Duration::from_millis(500)).await;
    }
    Err(anyhow::anyhow!("Timeout waiting for collection {} at {}", name, base_url))
}
