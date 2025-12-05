use clap::Parser;
use solidb::{
    cluster::{ClusterConfig, ReplicationService},
    create_router, StorageEngine,
};
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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
}

fn main() -> anyhow::Result<()> {
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

    // Build cluster configuration
    let cluster_config = ClusterConfig::new(args.node_id, args.peers, args.replication_port);
    tracing::info!("Node ID: {}", cluster_config.node_id);

    // Initialize storage engine with cluster config
    let storage = StorageEngine::with_cluster_config(&args.data_dir, cluster_config.clone())?;
    storage.initialize()?;
    tracing::info!("Storage engine initialized with _system database");

    // Keep a reference for shutdown
    let storage_for_shutdown = Arc::new(storage.clone());

    // Always start replication service (to accept incoming connections)
    tracing::info!(
        "Starting replication service on port {}",
        cluster_config.replication_port
    );
    let replication_service =
        ReplicationService::new(storage.clone(), cluster_config.clone(), &args.data_dir);
    let service_handle = replication_service.clone();
    tokio::spawn(async move {
        if let Err(e) = service_handle.start().await {
            tracing::error!("Replication service error: {}", e);
        }
    });

    // Create router with replication service
    let app = create_router(storage, Some(replication_service));

    // Start server with graceful shutdown
    let addr = format!("0.0.0.0:{}", args.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Server listening on {}", addr);

    // Handle shutdown signal
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
