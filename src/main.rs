use clap::Parser;
use solidb::{
    cluster::ClusterConfig,
    create_router, StorageEngine,
};
use std::sync::Arc;
use sysinfo::{Pid, System};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tokio::io::AsyncReadExt;

#[derive(Parser, Debug)]
#[command(name = "solidb")]
#[command(about = "SolidDB - A high-performance document database", long_about = None)]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value_t = 6745)]
    port: u16,

    /// Unique node identifier (auto-generated if not provided)
    #[arg(long)]
    node_id: Option<String>,

    /// Peer nodes to replicate with (e.g., --peer 192.168.1.2:6746)
    #[arg(long = "peer")]
    peers: Vec<String>,

    /// Port for inter-node replication traffic
    #[arg(long, default_value_t = 6746)]
    replication_port: u16,

    /// Data directory path
    #[arg(long, default_value = "./data")]
    data_dir: String,

    /// Run as a daemon (background process)
    #[arg(short, long)]
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

fn main() -> anyhow::Result<()> {
    // Load .env file if present (before parsing CLI args)
    let _ = dotenvy::dotenv();

    let args = Args::parse();

    // Handle daemonization before starting async runtime
    #[cfg(unix)]
    if args.daemon {
        use daemonize::Daemonize;
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
                            let still_running = unsafe {
                                libc::kill(pid, 0) == 0
                            };
                            
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
    let node_id = args.node_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let api_address = format!("127.0.0.1:{}", args.port); // Hostname resolution is complex, assuming loopback for now or args
    // In production we'd want actual IP, but for now this matches existing logic assumption
    let repl_address = format!("127.0.0.1:{}", args.replication_port);

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
        args.replication_port,
        args.keyfile.clone(),
    );

    let storage = StorageEngine::with_cluster_config(&args.data_dir, cluster_config.clone())?;
    storage.initialize()?;
    tracing::info!("Storage engine initialized");

    let storage_for_shutdown = Arc::new(storage.clone());

    // 3. Initialize Cluster Components (New Architecture)
    
    // Transport
    let transport = Arc::new(solidb::cluster::transport::TcpTransport::new(repl_address.clone()));
    
    // Cluster State
    let cluster_state = solidb::cluster::state::ClusterState::new(node_id.clone());
    
    // Cluster Manager
    // Replication Log (Create BEFORE Manager)
    let replication_log = Arc::new(solidb::replication::log::ReplicationLog::new(
        &args.data_dir,
        node_id.clone(),
    ).map_err(|e| anyhow::anyhow!("Failed to init replication log: {}", e))?);

    // Cluster Manager
    let cluster_manager = Arc::new(solidb::cluster::manager::ClusterManager::new(
        local_node.clone(),
        cluster_state,
        transport.clone(),
        Some(replication_log.clone()),
        Some(Arc::new(storage.clone())),
    ));



    // 4. Start Background Tasks
    
    // Cluster TCP Listener
    let listener = transport.listen().await?;
    let mgr_clone = cluster_manager.clone();
    tokio::spawn(async move {
        tracing::info!("Cluster listener started on {}", repl_address);
        while let Ok((mut socket, addr)) = listener.accept().await {
            let mgr = mgr_clone.clone();
            tokio::spawn(async move {
                // Simple one-shot message reading for now (connection per message)
                // In production, we'd want persistent connections/framing.
                let mut buf = Vec::new();
                if let Ok(_) = socket.read_to_end(&mut buf).await {
                    if let Ok(msg) = serde_json::from_slice(&buf) {
                        mgr.handle_message(msg).await;
                    } else {
                        tracing::warn!("Failed to deserialize cluster message from {}", addr);
                    }
                }
            });
        }
    });

    // Start Manager (Heartbeats etc)
    let mgr_clone2 = cluster_manager.clone();
    tokio::spawn(async move {
        mgr_clone2.start().await;
    });

    // Start Shard Cleanup Service
    let cleanup_config = solidb::sharding::cleanup::ShardCleanupConfig::default();
    let cleanup_service = solidb::sharding::cleanup::ShardCleanup::new(
        cleanup_config,
        Arc::new(storage.clone()), 
        cluster_manager.clone(),
    );
    tokio::spawn(async move {
        cleanup_service.start().await;
    });

    // Start Stats Collector
    let stats_storage = storage_for_shutdown.clone(); // Use the existing storage Arc
    // Create actual ShardCoordinator instance for stats (and potentially for handlers to use later)
    // For now we create a dedicated one or we could have created it earlier and passed it to AppState
    let stats_coordinator = Arc::new(solidb::sharding::coordinator::ShardCoordinator::new(
        stats_storage.clone(),
        cluster_manager.clone(),
    ));
    
    // STARTING STATS COLLECTOR
    // Re-using storage Arc. 
    let stats_collector = solidb::cluster::stats::ClusterStatsCollector::new(
        stats_storage,
        stats_coordinator,
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

    // Start Replication Worker
    let worker_log = replication_log.clone();
    let worker_transport = transport.clone();
    let worker_mgr = cluster_manager.clone();
    tokio::spawn(async move {
        let worker = solidb::replication::worker::ReplicationWorker::new(worker_log, worker_transport, worker_mgr);
        worker.start().await;
    });

    // Join Cluster if peers provided
    if !args.peers.is_empty() {
        let mgr_clone3 = cluster_manager.clone();
        let seeds = args.peers.clone();
        tokio::spawn(async move {
            // Wait for server to start
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            for seed in seeds {
                if let Err(e) = mgr_clone3.join_cluster(&seed).await {
                    tracing::warn!("Failed to join cluster via seed {}: {}", seed, e);
                } else {
                    tracing::info!("Sent join request to {}", seed);
                    break; // Only need one successful contact
                }
            }
        });
    }

    // 5. Create Router
    let app = create_router(
        storage,
        Some(cluster_manager),
        Some(replication_log),
        args.port
    );

    // 6. Start API Server
    let addr = format!("0.0.0.0:{}", args.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Server listening on {}", addr);

    let shutdown_storage = storage_for_shutdown.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown_storage))
        .await?;

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
