use clap::{Parser, Subcommand};
use solidb::server::multiplex::{ChannelListener, PeekedStream};
use solidb::{cluster::ClusterConfig, create_router, scripting::ScriptStats, StorageEngine};
use std::sync::Arc;
use sysinfo::{Pid, System};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(name = "solidb", version)]
#[command(about = "SolidDB - A high-performance document database", long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// Port to listen on
    #[arg(short, long, default_value_t = 6745)]
    port: u16,

    /// Unique node identifier (auto-generated if not provided)
    #[arg(long)]
    node_id: Option<String>,

    /// Peer nodes to replicate with (e.g., --peer 192.168.1.2:6746)
    #[arg(long = "peer")]
    peers: Vec<String>,

    /// Port for inter-node replication traffic (defaults to --port value for multiplexing)
    #[arg(long)]
    replication_port: Option<u16>,

    /// Data directory path
    #[arg(long, default_value = "./data")]
    data_dir: String,

    /// Run as a daemon (background process)
    #[arg(short = 'd', long)]
    daemon: bool,

    /// PID file path (used in daemon mode)
    #[arg(long, default_value = "./solidb.pid")]
    pid_file: String,

    /// Log file path (used in daemon mode)
    #[arg(long, default_value = "./solidb.log")]
    log_file: String,

    /// Optional keyfile for cluster node authentication
    #[arg(long)]
    keyfile: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Manage Lua scripts for custom API endpoints
    Scripts(solidb::cli::scripts::ScriptsArgs),
    /// Launch the Terminal User Interface
    Tui(solidb::cli::tui::TuiArgs),
}

fn main() -> anyhow::Result<()> {
    // Load .env file if present (before parsing CLI args)
    let _ = dotenvy::dotenv();

    let args = Args::parse();

    // Handle subcommands first (before daemonization)
    if let Some(command) = args.command {
        return match command {
            Command::Scripts(scripts_args) => solidb::cli::scripts::execute(scripts_args),
            Command::Tui(tui_args) => solidb::cli::tui::execute(tui_args),
        };
    }

    // Handle daemonization before starting async runtime
    #[cfg(unix)]
    if args.daemon {
        use solidb::daemon::Daemonize;
        use std::fs::File;
        use std::path::Path;

        // Check if PID file exists and kill existing process
        if Path::new(&args.pid_file).exists() {
            match std::fs::read_to_string(&args.pid_file) {
                Ok(pid_str) => {
                    if let Ok(pid) = pid_str.trim().parse::<i32>() {
                        // Verify process identity using sysinfo to prevent killing arbitrary processes
                        let mut sys = System::new_all();
                        sys.refresh_all();

                        let sys_pid = Pid::from(pid as usize);
                        if let Some(proc) = sys.process(sys_pid) {
                            if proc.name() != "solidb" {
                                eprintln!("SECURITY ERROR: Process with PID {} is named '{}', not 'solidb'. Refusing to kill potential mismatch.", pid, proc.name());
                                return Ok(());
                            }
                        }

                        eprintln!("Found existing server with PID {}. Stopping it...", pid);

                        // Send SIGTERM to gracefully stop the process
                        unsafe {
                            libc::kill(pid, libc::SIGTERM);
                        }

                        // Wait for the process to terminate (max 5 seconds)
                        for i in 0..50 {
                            std::thread::sleep(std::time::Duration::from_millis(100));

                            // Check if process is still running
                            let still_running = unsafe { libc::kill(pid, 0) == 0 };

                            if !still_running {
                                eprintln!("Previous server stopped successfully.");
                                break;
                            }

                            // After 3 seconds, send SIGKILL if still running
                            if i == 30 {
                                eprintln!("Process didn't stop gracefully, forcing shutdown...");
                                unsafe {
                                    libc::kill(pid, libc::SIGKILL);
                                }
                            }
                        }

                        // Remove the old PID file
                        let _ = std::fs::remove_file(&args.pid_file);
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Could not read PID file: {}", e);
                }
            }
        }

        let stdout = File::create(&args.log_file)?;
        let stderr = File::create(&args.log_file)?;

        let daemonize = Daemonize::new()
            .pid_file(&args.pid_file)
            .working_directory(".")
            .stdout(stdout)
            .stderr(stderr);

        match daemonize.start() {
            Ok(_) => {
                // We're now in the daemon process
            }
            Err(e) => {
                eprintln!("Error starting daemon: {}", e);
                std::process::exit(1);
            }
        }
    }

    #[cfg(not(unix))]
    if args.daemon {
        eprintln!("Daemon mode is only supported on Unix systems");
        std::process::exit(1);
    }

    // Start the async runtime
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async_main(args))
}

async fn async_main(args: Args) -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "solidb=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // 1. Setup Node Identity
    let node_id = args
        .node_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Default to multiplexing (same port) if replication_port is not set
    let replication_port = args.replication_port.unwrap_or(args.port);

    let api_address = format!("127.0.0.1:{}", args.port);
    let repl_address = format!("127.0.0.1:{}", replication_port);

    let local_node = solidb::cluster::node::Node::new(
        node_id.clone(),
        repl_address.clone(),
        api_address.clone(),
    );
    tracing::info!("Node ID: {}", local_node.id);
    tracing::info!("Replication Address: {}", local_node.address);
    tracing::info!("API Address: {}", local_node.api_address);

    // 2. Initialize Storage
    // We construct ClusterConfig just for StorageEngine compatibility if needed,
    // but ideally StorageEngine shouldn't depend on ClusterConfig anymore?
    // It uses it for _system._config.
    // Let's create a dummy or minimal config.
    let cluster_config = ClusterConfig::new(
        Some(node_id.clone()),
        args.peers.clone(),
        replication_port,
        args.keyfile.clone(),
    );

    let storage = StorageEngine::with_cluster_config(&args.data_dir, cluster_config.clone())?;
    storage.initialize()?;
    tracing::info!("Storage engine initialized");

    let storage_for_shutdown = Arc::new(storage.clone());

    // 3. Initialize Cluster Components (New Architecture)

    // Transport
    let transport = Arc::new(solidb::cluster::transport::TcpTransport::new(
        repl_address.clone(),
    ));

    // Cluster State
    let cluster_state = solidb::cluster::state::ClusterState::new(node_id.clone());

    // Cluster Manager
    // Replication Log (Create BEFORE Manager)
    // Replication Log (Create BEFORE Manager)
    let replication_log = Arc::new(
        solidb::sync::log::SyncLog::new(
            node_id.clone(),
            &args.data_dir,
            1000, // cache size
        )
        .map_err(|e| anyhow::anyhow!("Failed to init replication log: {}", e))?,
    );

    // Cluster Manager
    let cluster_manager = Arc::new(solidb::cluster::manager::ClusterManager::new(
        local_node.clone(),
        cluster_state,
        transport.clone(),
        Some(replication_log.clone()),
        Some(Arc::new(storage.clone())),
    ));

    // 4. Start Background Tasks

    // NOTE: In dual port mode, don't start a separate cluster listener because
    // the SyncWorker will bind to the replication port and handle protocol detection.
    // Cluster JSON messages will need to be routed through the sync protocol.
    // For now, cluster messages (JoinRequest etc) are sent directly via TcpTransport.connect_and_send()
    // which doesn't go through the listener.

    // Start Manager (Heartbeats etc)
    let mgr_clone2 = cluster_manager.clone();
    tokio::spawn(async move {
        mgr_clone2.start().await;
    });

    // Create ONE shared ShardCoordinator for ALL consumers to share the same shard table cache.
    // This is used by: stats collector, heal task, sync worker rebalancing, AND HTTP handlers (via routes).
    let shared_coordinator = Arc::new(solidb::sharding::coordinator::ShardCoordinator::new(
        storage_for_shutdown.clone(),
        Some(cluster_manager.clone()),
        Some(replication_log.clone()),
    ));

    // Start Stats Collector - uses shared coordinator
    let stats_storage = storage_for_shutdown.clone();
    let stats_collector = solidb::cluster::stats::ClusterStatsCollector::new(
        stats_storage,
        shared_coordinator.clone(), // Use shared coordinator
        cluster_manager.clone(),
    );
    tokio::spawn(async move {
        stats_collector.start().await;
    });

    // Start Health Monitor to detect dead nodes
    let health_config = solidb::cluster::health::HealthConfig::default();
    let health_state = cluster_manager.state().clone();
    let health_monitor = solidb::cluster::health::HealthMonitor::new(health_config, health_state);
    tokio::spawn(async move {
        health_monitor.start().await;
    });

    // Start Shard Healing Background Task (runs every 60 seconds) - uses shared coordinator
    // Creates new replicas when nodes fail to maintain replication factor
    // Also cleans up orphaned shards when node rejoins after being replaced

    // Clone for background task
    let healing_coordinator = shared_coordinator.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;

            // First, clean up any orphaned shards from previous node assignment
            if let Err(e) = healing_coordinator.cleanup_orphaned_shards().await {
                tracing::error!("Orphaned shard cleanup failed: {}", e);
            }

            // Then, heal shards by creating replicas on healthy nodes
            if let Err(e) = healing_coordinator.heal_shards().await {
                tracing::error!("Shard healing failed: {}", e);
            }
        }
    });

    // Start Blob Rebalance Worker (if cluster mode with multiple nodes)
    // The worker will check if rebalancing is needed based on node count
    let blob_rebalance_config = Arc::new(solidb::sharding::RebalanceConfig::default());
    let blob_worker = Arc::new(solidb::sharding::BlobRebalanceWorker::new(
        storage_for_shutdown.clone(),
        shared_coordinator.clone(),
        Some(cluster_manager.clone()),
        blob_rebalance_config,
    ));
    let blob_worker_start = blob_worker.clone();
    tokio::spawn(async move {
        blob_worker_start.start().await;
    });
    tracing::info!("BlobRebalanceWorker started");

    // Start Replication Worker
    let worker_log = replication_log.clone();
    let _worker_transport = transport.clone();
    let _worker_mgr = cluster_manager.clone();
    let worker_storage = Arc::new(storage.clone());
    let worker_node_id = node_id.clone();
    let worker_keyfile = args
        .keyfile
        .clone()
        .unwrap_or_else(|| "solidb.key".to_string());
    let worker_repl_addr = repl_address.clone();

    // Construct Sync Worker dependencies
    let sync_state = Arc::new(solidb::sync::state::SyncState::new(
        worker_storage.clone(),
        worker_node_id.clone(),
    ));

    let connection_pool = Arc::new(solidb::sync::transport::ConnectionPool::new(
        worker_node_id.clone(),
        worker_keyfile.clone(),
    ));

    let (_tx, worker_cmd_rx) = solidb::sync::worker::create_command_channel();
    let sync_config = solidb::sync::worker::SyncConfig::default();

    // Create base worker with ClusterManager for peer discovery
    // Use the shared coordinator for rebalancing (same cache as healing task)
    let sync_worker = solidb::sync::worker::SyncWorker::new(
        worker_storage,
        sync_state,
        connection_pool,
        worker_log,
        sync_config,
        worker_cmd_rx,
        worker_node_id,
        worker_keyfile,
        worker_repl_addr,
    )
    .with_cluster_manager(cluster_manager.clone())
    .with_shard_coordinator(shared_coordinator.clone());

    // Join Cluster if peers provided (as background task)
    if !args.peers.is_empty() {
        let mgr_clone3 = cluster_manager.clone();
        let seeds = args.peers.clone();

        // Use the shared coordinator for startup cleanup (same cache as healing/rebalancing)
        let startup_coordinator = shared_coordinator.clone();

        tokio::spawn(async move {
            // Wait for server to start
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            let mut joined = false;
            for seed in seeds {
                if let Err(e) = mgr_clone3.join_cluster(&seed).await {
                    tracing::warn!("Failed to join cluster via seed {}: {}", seed, e);
                } else {
                    tracing::info!("Sent join request to {}", seed);
                    joined = true;
                    break; // Only need one successful contact
                }
            }

            // If we joined the cluster, wait for shard tables to sync then cleanup orphaned data
            if joined {
                tracing::info!(
                    "Waiting for shard tables to sync before cleaning up orphaned shards..."
                );
                // Wait for initial sync and shard table discovery
                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

                // Trigger a rebalance first to load the latest shard tables from other nodes
                if let Err(e) = startup_coordinator.rebalance().await {
                    tracing::warn!("Startup rebalance failed: {}", e);
                }

                // Now clean up any shards that were reassigned while we were down
                match startup_coordinator.cleanup_orphaned_shards().await {
                    Ok(count) => {
                        if count > 0 {
                            tracing::info!(
                                "STARTUP: Cleaned up {} orphaned shard collections",
                                count
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!("STARTUP: Orphaned shard cleanup failed: {}", e);
                    }
                }

                // Trigger heal_shards to sync data for newly assigned shards
                // This ensures fresh nodes get their data immediately instead of waiting 60s
                match startup_coordinator.heal_shards().await {
                    Ok(count) => {
                        if count > 0 {
                            tracing::info!("STARTUP: Healed {} shard replicas", count);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("STARTUP: Shard healing failed: {}", e);
                    }
                }
            }
        });
    }

    // Initialize Script Stats
    let script_stats = Arc::new(ScriptStats::default());

    // Initialize Queue Management
    let queue_worker = Arc::new(solidb::queue::QueueWorker::new(
        Arc::new(storage.clone()),
        script_stats.clone(),
    ));

    let queue_worker_start = queue_worker.clone();
    tokio::spawn(async move {
        queue_worker_start.start().await;
    });

    // Initialize TTL Worker (background cleanup of expired documents)
    let ttl_worker = Arc::new(solidb::ttl::TtlWorker::new(Arc::new(storage.clone())));
    let ttl_worker_start = ttl_worker.clone();
    tokio::spawn(async move {
        ttl_worker_start.start().await;
    });

    // Initialize AI Recovery Worker (autonomous recovery for stalled tasks and agent health)
    let recovery_config = solidb::ai::RecoveryConfig::default();
    let recovery_worker = Arc::new(solidb::ai::RecoveryWorker::new(
        Arc::new(storage.clone()),
        "_system".to_string(), // Default database for AI operations
        recovery_config,
    ));
    let recovery_worker_start = recovery_worker.clone();
    tokio::spawn(async move {
        recovery_worker_start.start().await;
    });
    tracing::info!("AI Recovery Worker started");

    // Initialize Stream Manager
    let stream_manager = Arc::new(solidb::stream::StreamManager::new(Arc::new(
        storage.clone(),
    )));

    // Create Router - use the shared coordinator so all parts share the same shard table cache
    let app = create_router(
        storage,
        Some(cluster_manager.clone()),
        Some(replication_log.clone()),
        Some(shared_coordinator.clone()),
        Some(queue_worker),
        script_stats,
        Some(stream_manager),
        args.port,
    );

    let shutdown_storage = storage_for_shutdown.clone(); // prepare for signal

    // Determine launch mode
    // Determine launch mode
    if args.port == replication_port {
        tracing::info!("Starting in MULTIPLEXED mode on port {}", args.port);
        let addr = format!("0.0.0.0:{}", args.port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;

        let local_addr = listener.local_addr()?;

        // Channels for dispatch
        let (http_tx, http_rx) = mpsc::channel(100);
        let (sync_tx, sync_rx) = mpsc::channel(100);

        // 1. Spawn HTTP Server
        let channel_listener = ChannelListener::new(http_rx, local_addr);
        tokio::spawn(async move {
            if let Err(e) = axum::serve(channel_listener, app)
                .with_graceful_shutdown(shutdown_signal(shutdown_storage))
                .await
            {
                tracing::error!("HTTP server error: {}", e);
            }
        });

        // 2. Spawn Sync Worker (background mode)
        let sync_worker = sync_worker.with_incoming_channel(sync_rx);
        tokio::spawn(async move {
            sync_worker.run_background().await;
        });

        // 3. Spawn Driver Handler (native binary protocol)
        let driver_storage = storage_for_shutdown.clone();
        let driver_tx = solidb::driver::spawn_driver_handler(driver_storage);
        tracing::info!("Native driver protocol enabled on port {}", args.port);

        // 3. Dispatch Loop (Main Task) with shutdown handling
        let shutdown_signal_future = async {
            let ctrl_c = async {
                tokio::signal::ctrl_c()
                    .await
                    .expect("failed to install Ctrl+C handler");
            };

            #[cfg(unix)]
            let terminate = async {
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("failed to install signal handler")
                    .recv()
                    .await;
            };

            #[cfg(not(unix))]
            let terminate = std::future::pending::<()>();

            tokio::select! {
                _ = ctrl_c => {},
                _ = terminate => {},
            }
        };

        tokio::pin!(shutdown_signal_future);

        loop {
            tokio::select! {
                _ = &mut shutdown_signal_future => {
                    tracing::info!("Shutdown signal received in multiplexed mode, stopping...");
                    storage_for_shutdown.flush_all_stats();
                    tracing::info!("Shutdown complete");
                    std::process::exit(0);
                }
                accept_result = listener.accept() => {
                    let (mut stream, addr) = match accept_result {
                        Ok(conn) => conn,
                        Err(e) => {
                            tracing::error!("Accept error: {}", e);
                            continue;
                        }
                    };

                    let http_tx = http_tx.clone();
                    let sync_tx = sync_tx.clone();
                    let driver_tx = driver_tx.clone();
                    let connection_mgr = cluster_manager.clone();

                    tokio::spawn(async move {
                        // Read initial bytes to determine protocol
                        let mut buf = vec![0u8; 14];
                        let n = stream.read(&mut buf).await.unwrap_or(0);

                        let peeked_data = buf[..n].to_vec();

                        // Detection logic - check magic headers first
                        // Check for Sync Protocol: "solidb-sync-v1"
                        if &peeked_data == b"solidb-sync-v1" {
                            // For sync traffic, pass the raw stream - the magic header has been consumed
                            // and verified, so we don't need to put it back in a PeekedStream
                            if sync_tx.send((Box::new(stream), addr.to_string())).await.is_err() {
                                 tracing::error!("Sync worker channel closed");
                            }
                        }
                        // Check for Native Driver Protocol: "solidb-drv-v1\0"
                        else if &peeked_data == b"solidb-drv-v1\0" {
                            // For driver traffic, pass the raw stream to the driver handler
                            if driver_tx.send((stream, addr.to_string())).await.is_err() {
                                 tracing::error!("Driver handler channel closed");
                            }
                        }
                        // Check for Cluster JSON Messages
                        else if peeked_data.first() == Some(&b'{') {
                            // Cluster Message (JSON) - need peeked bytes for parsing
                            let peeked_stream = PeekedStream::new(stream, peeked_data.clone());
                            let mgr = connection_mgr.clone();
                            tokio::spawn(async move {
                                 let mut buf = Vec::new();
                                 let mut stream = peeked_stream;
                                 if stream.read_to_end(&mut buf).await.is_ok() {
                                    if let Ok(msg) = serde_json::from_slice(&buf) {
                                        mgr.handle_message(msg).await;
                                    } else {
                                        tracing::warn!("Failed to deserialize cluster message from {}", addr);
                                    }
                                 }
                            });
                        } else {
                            // HTTP traffic - need peeked bytes for HTTP parsing
                            let peeked_stream = PeekedStream::new(stream, peeked_data.clone());
                            if http_tx.send((peeked_stream, addr)).await.is_err() {
                                 tracing::error!("HTTP server channel closed");
                            }
                        }
                    });
                }
            }
        }
    } else {
        tracing::info!(
            "Starting in DUAL PORT mode (API: {}, Sync: {})",
            args.port,
            replication_port
        );

        // 1. Spawn Sync Worker (standard mode)
        tokio::spawn(async move {
            sync_worker.run().await;
        });

        // 2. Serve HTTP (standard mode)
        let addr = format!("0.0.0.0:{}", args.port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!("Server listening on {}", addr);

        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal(shutdown_storage))
            .await?;
    }

    Ok(())
}

async fn shutdown_signal(storage: Arc<StorageEngine>) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received, flushing stats...");
    storage.flush_all_stats();
    tracing::info!("Shutdown complete");
}
