use clap::Parser;
use solidb::{create_router, StorageEngine, cluster::{ClusterConfig, ReplicationService}};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use std::sync::Arc;

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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "solidb=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Build cluster configuration
    let cluster_config = ClusterConfig::new(
        args.node_id,
        args.peers,
        args.replication_port,
    );
    tracing::info!("Node ID: {}", cluster_config.node_id);

    // Initialize storage engine with cluster config
    let storage = StorageEngine::with_cluster_config(&args.data_dir, cluster_config.clone())?;
    storage.initialize()?;
    tracing::info!("Storage engine initialized with _system database");

    // Keep a reference for shutdown
    let storage_for_shutdown = Arc::new(storage.clone());

    // Always start replication service (to accept incoming connections)
    tracing::info!("Starting replication service on port {}", cluster_config.replication_port);
    let replication_service = ReplicationService::new(storage.clone(), cluster_config.clone(), &args.data_dir);
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
