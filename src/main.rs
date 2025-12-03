use solidb::{create_router, StorageEngine};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

    // Create router
    let app = create_router(storage);

    // Start server
    let addr = "0.0.0.0:6745";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
