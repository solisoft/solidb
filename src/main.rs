use clap::Parser;
use solidb::{create_router, StorageEngine};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(name = "solidb")]
#[command(about = "SolidDB - A high-performance document database", long_about = None)]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value_t = 6745)]
    port: u16,
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

    // Initialize storage engine
    let storage = StorageEngine::new("./data")?;
    storage.initialize()?;
    tracing::info!("Storage engine initialized with _system database");

    // Keep a reference for shutdown
    let storage_for_shutdown = Arc::new(storage.clone());

    // Create router
    let app = create_router(storage);

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
