// Use jemalloc instead of the system (glibc) allocator. RocksDB + many threads
// make glibc's per-thread arenas pin RSS at the high-water mark; jemalloc keeps
// RSS tracking the actual working set and returns freed memory to the OS.
#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// jemalloc tuning, read by the allocator during its own initialization —
/// before `main`, which is why this is a link-time symbol and not a runtime
/// mallctl call. tikv-jemalloc-sys exports `malloc_conf` under its `_rjem_`
/// prefix and leaves it weak, so this strong definition wins at link time.
///
/// `background_thread` is off by default, and without it an arena only purges
/// its dirty pages while some thread keeps allocating *in that arena*. Under a
/// burst of concurrent queries every Tokio worker grows its own arena; when the
/// burst ends those threads go idle and their freed pages stay resident
/// forever. Measured on a dev box with 89 databases: a 3200-query burst at 32×
/// concurrency took RSS from 662 MB to 1296 MB and was still at 954 MB five
/// minutes later. With a background purger and a 2s decay the same burst came
/// back to 625 MB, and the idle baseline dropped ~18%.
#[cfg(not(target_env = "msvc"))]
#[allow(non_upper_case_globals)]
#[export_name = "_rjem_malloc_conf"]
pub static malloc_conf: &[u8; 63] =
    b"background_thread:true,dirty_decay_ms:2000,muzzy_decay_ms:2000\0";

/// Report the allocator options jemalloc actually started with.
///
/// The tuning above is applied through a link-time symbol, so a rename in
/// tikv-jemalloc-sys (or a build that drops the prefix) would make it a no-op
/// with nothing to show for it. Reading the values back turns that from
/// invisible into a warning in the log.
#[cfg(not(target_env = "msvc"))]
fn log_allocator_tuning() {
    use tikv_jemalloc_ctl::{opt, raw};

    let background = opt::background_thread::read().unwrap_or(false);
    // Decay intervals have no typed accessor in tikv-jemalloc-ctl; read the
    // mallctl directly. Both are `ssize_t` (-1 means "never decay").
    let dirty_ms = unsafe { raw::read::<isize>(b"opt.dirty_decay_ms\0") }.unwrap_or(-1);
    let muzzy_ms = unsafe { raw::read::<isize>(b"opt.muzzy_decay_ms\0") }.unwrap_or(-1);

    if background {
        tracing::info!(
            "jemalloc: background_thread=true, dirty_decay_ms={}, muzzy_decay_ms={}",
            dirty_ms,
            muzzy_ms
        );
    } else {
        tracing::warn!(
            "jemalloc: background_thread is OFF (dirty_decay_ms={}) — pages freed by \
             threads that then go idle will stay resident. The `_rjem_malloc_conf` \
             symbol in main.rs is not reaching the allocator.",
            dirty_ms
        );
    }
}

#[cfg(target_env = "msvc")]
fn log_allocator_tuning() {}

use clap::{Parser, Subcommand};
use solidb::server::multiplex::{ChannelListener, PeekedStream};
use solidb::{cluster::ClusterConfig, create_router, scripting::ScriptStats, StorageEngine};
use std::sync::Arc;
use sysinfo::{Pid, System};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio::time::Duration;
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

    /// Address to bind the listeners to. Defaults to 0.0.0.0 (all
    /// interfaces); use 127.0.0.1 to restrict the node to loopback, e.g.
    /// behind a reverse proxy. Falls back to the SOLIDB_HOST environment
    /// variable when the flag is absent.
    #[arg(long)]
    host: Option<String>,

    /// Unique node identifier (auto-generated if not provided)
    #[arg(long)]
    node_id: Option<String>,

    /// The address peers should use to reach this node.
    ///
    /// Either a bare host (`10.0.0.1`) or a host with a port
    /// (`10.0.0.1:6746`). With a port, that is the port peers dial for
    /// replication; without one, --replication-port is appended.
    ///
    /// Defaults to --host. Required when --host is 0.0.0.0, because a node
    /// cannot guess which of its addresses a peer can route to, and guessing
    /// loopback means advertising "me" to everyone.
    #[arg(long)]
    advertise: Option<String>,

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

    /// OpenTelemetry OTLP endpoint (e.g., http://localhost:4317)
    /// If not set, tracing is disabled
    #[arg(long)]
    otlp_endpoint: Option<String>,

    /// Disable the sync log entirely. Useful for single-node dev setups
    /// where there are no peers — avoids doubling write storage.
    /// WARNING: never set this on a node that participates in replication.
    #[arg(long)]
    no_sync_log: bool,

    /// Use the low-memory storage profile. Shrinks per-CF memtables, adds a
    /// global memtable budget, caps open files, and stores index/filter
    /// blocks in the bounded block cache. Intended for dev boxes with many
    /// idle collections; trades some throughput for much lower RAM.
    #[arg(long)]
    dev: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Manage Lua scripts for custom API endpoints
    Scripts(solidb::cli::scripts::ScriptsArgs),
    /// Launch the Terminal User Interface
    Tui(solidb::cli::tui::TuiArgs),
    /// Update SoliDB to the latest release
    Update,
}

fn main() -> anyhow::Result<()> {
    // Install JWT crypto provider (aws-lc backend; avoids vulnerable `rsa` crate)
    let _ = jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER.install_default();

    // Load .env file if present (before parsing CLI args)
    let _ = dotenvy::dotenv();

    let args = Args::parse();

    // Handle subcommands first (before daemonization)
    if let Some(command) = args.command {
        return match command {
            Command::Scripts(scripts_args) => solidb::cli::scripts::execute(scripts_args),
            Command::Tui(tui_args) => solidb::cli::tui::execute(tui_args),
            Command::Update => solidb::cli::update::execute(),
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
                            let proc_name = proc.name().to_string_lossy();
                            if proc_name != "solidb" {
                                eprintln!("SECURITY ERROR: Process with PID {} is named '{}', not 'solidb'. Refusing to kill potential mismatch.", pid, proc_name);
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

/// Address the listeners bind to: `--host`, then `SOLIDB_HOST`, then
/// 0.0.0.0 (all interfaces, the historical default).
fn bind_host(args: &Args) -> String {
    args.host
        .clone()
        .or_else(|| std::env::var("SOLIDB_HOST").ok())
        .unwrap_or_else(|| "0.0.0.0".to_string())
}

async fn async_main(args: Args) -> anyhow::Result<()> {
    let host = bind_host(&args);
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

    // The address peers are told to reach this node on.
    //
    // Both of these were hardcoded to `127.0.0.1`, which means a node in a
    // multi-machine cluster advertised itself as "me" to every peer. The
    // cluster came up, logged nothing unusual, and replicated nothing —
    // measured on two Scaleway instances. Same shape as the SoliKV gossip
    // server, which bound loopback unconditionally for the same reason: the
    // address a node *listens* on and the address it *advertises* are
    // different questions, and only one of them can be guessed.
    //
    // `--host 0.0.0.0` is precisely the case that cannot be guessed, so with
    // peers configured it is refused rather than defaulted. A cluster that
    // silently cannot replicate is worse than one that will not start.
    let advertise = args
        .advertise
        .clone()
        .or_else(|| args.host.clone())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    if let Err(reason) = check_advertise(&advertise, &args.peers) {
        eprintln!("ERROR: {reason}");
        eprintln!(
            "       Set --advertise to the address peers can reach this node on, or bind \
             --host to that address."
        );
        std::process::exit(1);
    }

    // `--advertise` may already carry a port, and appending a second one used to
    // produce `10.0.0.1:6746:6746`. That address fails to resolve, the failure
    // was on a discarded `Result`, and the result was a cluster that looked
    // healthy from the seed while the joining node's own node list stayed empty
    // — the join succeeded, the answer never arrived.
    //
    // Accepted in both forms rather than refused, because both are things people
    // write: the flag's own help says "the address peers should use", which reads
    // as host:port, and its default is `--host`, which is a bare host.
    let api_address = with_port(&advertise, args.port);
    let repl_address = with_port(&advertise, replication_port);

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

    // Clustered deployments must authenticate inter-node traffic. Without a
    // keyfile, replication and cluster-control messages would be accepted
    // from anyone who can reach the port (HMAC is silently skipped), so
    // refuse to start rather than run an open cluster.
    if !args.peers.is_empty() && cluster_config.keyfile.is_none() {
        anyhow::bail!(
            "Cluster peers are configured but no keyfile is available. \
             Create a shared secret of 32 cryptographically random bytes, hex-encoded, \
             and pass it with --keyfile (the same file on every node). \
             Unix: `openssl rand -hex 32 > solidb.key`. PowerShell: \
             `$b=[byte[]]::new(32);[Security.Cryptography.RandomNumberGenerator]::Fill($b);\
             [BitConverter]::ToString($b).Replace('-','').ToLower() \
             | Out-File -Encoding ascii solidb.key`. \
             Refusing to start an unauthenticated cluster."
        );
    }
    if args.peers.is_empty() && cluster_config.keyfile.is_none() {
        tracing::warn!(
            "No cluster keyfile configured: replication and cluster ports accept \
             unauthenticated connections. Set --keyfile before adding peers."
        );
    }

    log_allocator_tuning();

    // Select the RocksDB memory/tuning profile BEFORE constructing the engine
    // (the shared block cache and CF options are built lazily on first use).
    use solidb::storage::engine::{set_engine_profile, EngineProfile};
    if args.dev {
        set_engine_profile(EngineProfile::dev());
        tracing::info!("Storage profile: dev (low-memory)");
    } else {
        set_engine_profile(EngineProfile::prod());
    }

    let storage = StorageEngine::with_cluster_config(&args.data_dir, cluster_config.clone())?;
    storage.initialize()?;
    tracing::info!("Storage engine initialized");

    let storage_for_shutdown = Arc::new(storage.clone());

    // 3. Initialize Cluster Components (New Architecture)

    // Transport
    let transport = Arc::new(solidb::cluster::transport::TcpTransport::new(
        repl_address.clone(),
        cluster_config.keyfile.clone(),
    ));

    // Cluster State
    let cluster_state = solidb::cluster::state::ClusterState::new(node_id.clone());

    // Cluster Manager
    // Replication Log (Create BEFORE Manager)
    // Replication Log (Create BEFORE Manager)
    let replication_log = Arc::new(
        solidb::sync::log::SyncLog::new_with_options(
            node_id.clone(),
            &args.data_dir,
            1000, // cache size
            args.no_sync_log,
        )
        .map_err(|e| anyhow::anyhow!("Failed to init replication log: {}", e))?,
    );
    if args.no_sync_log {
        tracing::warn!(
            "Sync log is DISABLED via --no-sync-log; do not use this flag on a node participating in replication"
        );
    }

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
        // Exponential backoff on consecutive failures: with an unreachable
        // peer, a fixed 5s cadence turns into a retry storm of failed
        // outbound connections and log spam. Back off up to 5 minutes and
        // reset as soon as a cycle succeeds.
        let base = std::time::Duration::from_secs(5);
        let max_backoff = std::time::Duration::from_secs(300);
        let mut delay = base;
        loop {
            tokio::time::sleep(delay).await;

            let mut failed = false;

            // First, clean up any orphaned shards from previous node assignment
            if let Err(e) = healing_coordinator.cleanup_orphaned_shards().await {
                tracing::error!("Orphaned shard cleanup failed: {}", e);
                failed = true;
            }

            // Then, heal shards by creating replicas on healthy nodes
            if let Err(e) = healing_coordinator.heal_shards().await {
                tracing::error!("Shard healing failed: {}", e);
                failed = true;
            }

            delay = if failed {
                (delay * 2).min(max_backoff)
            } else {
                base
            };
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

    // The sender was discarded as `_tx`, which made every `SyncCommand`
    // unreachable — including `RequestFullSync`, the only thing that gives a
    // joining node the data that existed before it arrived. `request_full_sync`
    // was implemented, its handler was implemented, and nothing could call it.
    //
    // The visible symptom on two real nodes: the join succeeded, replication
    // carried new writes, and `_admins` — created before the peer joined —
    // never arrived, so the peer answered 401 to everything for good.
    let (sync_cmd_tx, worker_cmd_rx) = solidb::sync::worker::create_command_channel();
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
        let full_sync_tx = sync_cmd_tx.clone();

        // Use the shared coordinator for startup cleanup (same cache as healing/rebalancing)
        let startup_coordinator = shared_coordinator.clone();

        tokio::spawn(async move {
            // Wait for server to start
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            let mut joined = false;
            let mut joined_via = String::new();
            for seed in seeds {
                if let Err(e) = mgr_clone3.join_cluster(&seed).await {
                    tracing::warn!("Failed to join cluster via seed {}: {}", seed, e);
                } else {
                    tracing::info!("Sent join request to {}", seed);
                    joined_via = seed.clone();
                    joined = true;
                    break; // Only need one successful contact
                }
            }

            // If we joined the cluster, wait for shard tables to sync then cleanup orphaned data
            if joined {
                // Ask for everything that existed before this node did.
                //
                // Replication carries writes forward; it has no opinion about
                // the past. Without this the node holds whatever arrives from
                // now on and nothing else — which for a fresh peer means no
                // `_admins`, and therefore no way to authenticate against it.
                tracing::info!("Requesting a full sync from {joined_via}");
                if let Err(e) = full_sync_tx
                    .send(solidb::sync::worker::SyncCommand::RequestFullSync {
                        peer_addr: joined_via.clone(),
                    })
                    .await
                {
                    tracing::error!(
                        "Could not request a full sync from {joined_via}: {e}. This node will \
                         only receive writes made from now on, and will not have the data that \
                         already existed."
                    );
                }

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

    // Create HTTP client with connection pooling for better performance
    let http_client = Arc::new(
        reqwest::Client::builder()
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(60))
            .tcp_keepalive(Duration::from_secs(60))
            .tcp_nodelay(true)
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client"),
    );
    tracing::info!("HTTP client with connection pooling initialized");

    // Initialize global HTTP client for use throughout the application
    solidb::storage::http_client::init_http_client(http_client.as_ref().clone());

    // Create Router - use the shared coordinator so all parts share the same shard table cache
    let app = create_router(
        storage,
        Some(cluster_manager.clone()),
        Some(replication_log.clone()),
        Some(shared_coordinator.clone()),
        Some(queue_worker),
        script_stats,
        Some(stream_manager),
        Some(blob_worker),
        args.port,
    );

    let shutdown_storage = storage_for_shutdown.clone(); // prepare for signal

    // Determine launch mode
    // Determine launch mode
    if args.port == replication_port {
        tracing::info!("Starting in MULTIPLEXED mode on port {}", args.port);
        let addr = format!("{}:{}", host, args.port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;

        let local_addr = listener.local_addr()?;

        // Channels for dispatch. Sized to absorb burst traffic between
        // accept and the protocol worker without back-pressuring the
        // accept loop. 8192 is enough to hold several seconds of typical
        // in-flight requests even at 10K req/s. If the receiver is
        // genuinely saturated, dropping new accepts is preferable to
        // head-of-line blocking behind an `await` on `send()`.
        let (http_tx, http_rx) = mpsc::channel(8192);
        let (sync_tx, sync_rx) = mpsc::channel(8192);

        // 1. Spawn HTTP Server
        //
        // Driven via hyper_util's auto Builder rather than `axum::serve` so we
        // can set an HTTP/1 header-read timeout. Without it, a client that
        // opens a keep-alive connection and then sends partial (or no) request
        // headers parks a server task indefinitely — the keep-alive analogue of
        // the unbounded protocol-sniff read bounded above. Graceful shutdown is
        // preserved via `GracefulShutdown`.
        let channel_listener = ChannelListener::new(http_rx, local_addr);
        let http_shutdown = shutdown_signal(shutdown_storage);
        tokio::spawn(async move {
            use axum::serve::Listener;
            use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
            use hyper_util::server::conn::auto::Builder as HttpConnBuilder;
            use hyper_util::server::graceful::GracefulShutdown;
            use hyper_util::service::TowerToHyperService;

            let mut listener = channel_listener;
            let graceful = GracefulShutdown::new();
            tokio::pin!(http_shutdown);

            // Provide `ConnectInfo<SocketAddr>` to handlers (e.g. login rate
            // limiting keys on the real peer address rather than spoofable
            // proxy headers).
            let mut make_service =
                app.into_make_service_with_connect_info::<std::net::SocketAddr>();

            loop {
                let (io, addr) = tokio::select! {
                    conn = listener.accept() => conn,
                    _ = &mut http_shutdown => {
                        tracing::info!("HTTP server received shutdown signal, draining connections");
                        break;
                    }
                };

                let mut builder = HttpConnBuilder::new(TokioExecutor::new());
                // Bound the time a connection may take to send a complete set
                // of request headers (defends against slow/half-open clients).
                // `header_read_timeout` requires a registered timer.
                builder
                    .http1()
                    .timer(TokioTimer::new())
                    .header_read_timeout(Duration::from_secs(30));
                // Infallible: IntoMakeServiceWithConnectInfo is always ready
                // and its error type is Infallible.
                let tower_service = {
                    use tower::Service;
                    match make_service.call(addr).await {
                        Ok(svc) => svc,
                        Err(never) => match never {},
                    }
                };
                let service = TowerToHyperService::new(tower_service);
                let conn = builder
                    .serve_connection_with_upgrades(TokioIo::new(io), service)
                    .into_owned();
                let watched = graceful.watch(conn);
                tokio::spawn(async move {
                    if let Err(e) = watched.await {
                        tracing::debug!("HTTP connection error: {}", e);
                    }
                });
            }

            graceful.shutdown().await;
        });

        // 2. Spawn Sync Worker (background mode)
        let sync_worker = sync_worker.with_incoming_channel(sync_rx);
        tokio::spawn(async move {
            sync_worker.run_background().await;
        });

        // 3. Spawn Driver Handler (native binary protocol)
        let driver_storage = storage_for_shutdown.clone();
        let driver_tx =
            solidb::driver::spawn_driver_handler(driver_storage, Some(replication_log.clone()));
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
                    let cluster_secret = cluster_config.keyfile.clone();

                    tokio::spawn(async move {
                        // Read initial bytes to determine protocol.
                        //
                        // Bound this read: a connection that is accepted but
                        // never sends bytes (half-open keep-alive reuse, a
                        // dead pooled connection, a port scanner) would
                        // otherwise park this task forever holding the socket.
                        // That unbounded wait is a primary source of the
                        // intermittent "idle pending" stalls seen in prod.
                        let mut buf = vec![0u8; 14];
                        let n = match tokio::time::timeout(
                            std::time::Duration::from_secs(10),
                            stream.read(&mut buf),
                        )
                        .await
                        {
                            Ok(Ok(n)) => n,
                            Ok(Err(_)) => 0,
                            Err(_) => {
                                tracing::warn!(
                                    layer = "solidb_detect",
                                    peer = %addr,
                                    "protocol detection read timed out; dropping connection"
                                );
                                return;
                            }
                        };

                        let peeked_data = buf[..n].to_vec();

                        // Detection logic - check magic headers first.
                        // Dispatch via `dispatch_or_drop` (try_send) so a
                        // saturated protocol worker doesn't back-pressure the
                        // accept loop.
                        // Check for Sync Protocol: "solidb-sync-v1"
                        if &peeked_data == b"solidb-sync-v1" {
                            // For sync traffic, pass the raw stream - the magic header has been consumed
                            // and verified, so we don't need to put it back in a PeekedStream
                            let sync_stream: solidb::sync::transport::SyncStream = Box::new(stream);
                            dispatch_or_drop(&sync_tx, (sync_stream, addr.to_string()), "sync", &addr);
                        }
                        // Check for Native Driver Protocol: "solidb-drv-v1\0"
                        else if &peeked_data == b"solidb-drv-v1\0" {
                            // For driver traffic, pass the raw stream to the driver handler
                            dispatch_or_drop(&driver_tx, (stream, addr.to_string()), "driver", &addr);
                        }
                        // Check for Cluster JSON Messages
                        else if peeked_data.first() == Some(&b'{') {
                            // Cluster Message (JSON) - need peeked bytes for parsing
                            let peeked_stream = PeekedStream::new(stream, peeked_data.clone());
                            let mgr = connection_mgr.clone();
                            tokio::spawn(async move {
                                // Bound both the size (cluster control messages are
                                // small; an unbounded read_to_end lets anyone OOM the
                                // node by streaming data) and the time (a held-open
                                // connection would park this task forever).
                                let mut buf = Vec::new();
                                let mut stream = tokio::io::AsyncReadExt::take(
                                    peeked_stream,
                                    (solidb::cluster::transport::MAX_CLUSTER_MESSAGE_SIZE + 1) as u64,
                                );
                                let read = tokio::time::timeout(
                                    std::time::Duration::from_secs(10),
                                    stream.read_to_end(&mut buf),
                                )
                                .await;
                                match read {
                                    Ok(Ok(_)) if buf.len() <= solidb::cluster::transport::MAX_CLUSTER_MESSAGE_SIZE => {
                                        // When a keyfile is configured, only HMAC-signed
                                        // messages are accepted — membership changes and
                                        // rebalances must not be attacker-injectable.
                                        match solidb::cluster::transport::open_cluster_message(
                                            &buf,
                                            cluster_secret.as_deref(),
                                        ) {
                                            Ok(msg) => mgr.handle_message(msg).await,
                                            Err(e) => {
                                                tracing::warn!(
                                                    "Rejected cluster message from {}: {}",
                                                    addr,
                                                    e
                                                );
                                            }
                                        }
                                    }
                                    Ok(Ok(_)) => {
                                        tracing::warn!(
                                            "Cluster message from {} exceeds size limit, dropped",
                                            addr
                                        );
                                    }
                                    _ => {
                                        tracing::warn!(
                                            "Cluster message read from {} failed or timed out",
                                            addr
                                        );
                                    }
                                }
                            });
                        } else {
                            // HTTP traffic - need peeked bytes for HTTP parsing
                            let peeked_stream = PeekedStream::new(stream, peeked_data.clone());
                            dispatch_or_drop(&http_tx, (peeked_stream, addr), "HTTP", &addr);
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
        let addr = format!("{}:{}", host, args.port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!("Server listening on {}", addr);

        axum::serve(
            listener,
            // Provide `ConnectInfo<SocketAddr>` (login rate limiting keys on
            // the real peer address).
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal(shutdown_storage))
        .await?;
    }

    Ok(())
}

/// Hand a freshly accepted connection to a protocol worker without awaiting:
/// if the dispatch channel is full we drop the connection (the client will
/// see a closed connection and retry) rather than back-pressure the accept
/// loop behind an `await` on `send()`.
fn dispatch_or_drop<T>(tx: &mpsc::Sender<T>, item: T, what: &str, peer: &std::net::SocketAddr) {
    match tx.try_send(item) {
        Ok(()) => {}
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            tracing::warn!(peer = %peer, "{} dispatch channel full, dropping connection", what);
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            tracing::error!("{} dispatch channel closed", what);
        }
    }
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

/// Whether this node can be reached at what it is about to advertise.
///
/// The rule is about the *pair*, not the address alone. Advertising loopback to a
/// peer on another machine is meaningless — that peer reads it as itself, the
/// cluster comes up, and nothing replicates while nothing looks wrong; measured
/// on two Scaleway instances. But when every peer is *also* loopback, the whole
/// cluster is on one host, and `127.0.0.1:6746` and `127.0.0.1:6747` are two
/// genuinely different endpoints.
///
/// An earlier version refused loopback whenever `--peer` was set, which made a
/// single-host cluster unstartable — and single-host is how anyone first tries
/// this. It survived only because a related bug let the `host:port` form slip
/// past the check entirely.
fn check_advertise(advertise: &str, peers: &[String]) -> Result<(), String> {
    if peers.is_empty() {
        return Ok(());
    }
    if !is_unroutable_advertise(advertise) {
        return Ok(());
    }

    // The unspecified address is never advertisable, on one host or a hundred:
    // it is a bind address, and no peer can dial "every interface".
    let host = advertised_host(advertise);
    let unspecified = host.is_empty()
        || host == "0.0.0.0"
        || host == "::"
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_unspecified())
            .unwrap_or(false);
    if unspecified {
        return Err(format!(
            "--peer is set but this node would advertise itself as {advertise:?}, which is a \
             bind address, not one a peer can dial"
        ));
    }

    // Loopback, then. Fine if and only if every peer is loopback too.
    let elsewhere: Vec<&String> = peers
        .iter()
        .filter(|peer| !is_unroutable_advertise(peer))
        .collect();
    if elsewhere.is_empty() {
        return Ok(());
    }
    Err(format!(
        "--peer is set but this node would advertise itself as {advertise:?}, which every peer \
         reads as its own loopback — and {} is not on this host",
        elsewhere[0]
    ))
}

/// Attaches a port to an advertised address, unless it already has one.
///
/// `--advertise` is written both ways in practice, and appending unconditionally
/// turned `10.0.0.1:6746` into `10.0.0.1:6746:6746`. That does not resolve, and
/// the one place it mattered swallowed the error: a joining node was admitted by
/// the seed and never received the answer naming its peers, so the seed's view
/// was complete and the joiner's was empty. Nothing logged, nothing failed.
///
/// "Already has one" is decided by parsing, not by counting colons. A bare IPv6
/// address is full of them (`::1`), and treating its last group as a port is the
/// same class of mistake in the other direction.
fn with_port(advertise: &str, port: u16) -> String {
    let host = advertise.trim();
    // `[::1]:6746` and `10.0.0.1:6746` both parse; `::1` and `10.0.0.1` do not.
    if host.parse::<std::net::SocketAddr>().is_ok() {
        return host.to_string();
    }
    // A bracketed IPv6 host with no port: `[::1]`.
    if host.starts_with('[') && host.ends_with(']') {
        return format!("{host}:{port}");
    }
    // A bare IPv6 address needs brackets before a port can be appended, or the
    // result is unparseable in a way that only shows up at connect time.
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        return format!("[{host}]:{port}");
    }
    // A hostname with a port, which no IP parser will accept: `db1.example.com:6746`.
    if let Some((name, tail)) = host.rsplit_once(':') {
        if !name.is_empty() && !name.contains(':') && tail.parse::<u16>().is_ok() {
            return host.to_string();
        }
    }
    format!("{host}:{port}")
}

/// Whether an advertised address is one a peer could never use.
///
/// Loopback tells a peer to talk to itself. The unspecified address is the one
/// that looks harmless in a systemd unit and means "every interface" — a fine
/// thing to *bind* and a meaningless thing to *advertise*.
///
/// Takes the value as written, so it has to cope with a port being present.
fn is_unroutable_advertise(address: &str) -> bool {
    // The port is stripped first. Without this, `--advertise 127.0.0.1:6746`
    // stopped parsing as an IP address and slipped straight through the guard —
    // the guard caught the bare form and missed the one an operator following the
    // error message would actually type.
    let host = advertised_host(address);
    if host.is_empty()
        || host == "0.0.0.0"
        || host == "::"
        || host.eq_ignore_ascii_case("localhost")
    {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback() || ip.is_unspecified())
        .unwrap_or(false)
}

/// The host part of an advertised address, with any port and brackets removed.
fn advertised_host(address: &str) -> String {
    let value = address.trim();
    if let Ok(socket) = value.parse::<std::net::SocketAddr>() {
        return socket.ip().to_string();
    }
    if let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
        return inner.to_string();
    }
    // Bare IPv6 keeps all its colons; only split when the tail is a port and the
    // head holds none.
    if let Some((name, tail)) = value.rsplit_once(':') {
        if !name.is_empty() && !name.contains(':') && tail.parse::<u16>().is_ok() {
            return name.to_string();
        }
    }
    value.to_string()
}

#[cfg(test)]
mod advertise_tests {
    use super::{advertised_host, check_advertise, is_unroutable_advertise, with_port};

    #[test]
    fn loopback_cannot_be_advertised_to_a_peer() {
        // The value that shipped: every node told every peer to talk to
        // itself, and the cluster replicated nothing while logging nothing
        // unusual. Measured on two Scaleway instances.
        assert!(is_unroutable_advertise("127.0.0.1"));
        assert!(is_unroutable_advertise("::1"));
        // The written-out form too: it does not parse as an address, so the
        // IP check never sees it, and advertising it is the same mistake.
        assert!(is_unroutable_advertise("localhost"));
    }

    #[test]
    fn the_unspecified_address_cannot_be_advertised_either() {
        // Fine to bind, meaningless to advertise — and it is the value that
        // looks harmless in a systemd unit.
        assert!(is_unroutable_advertise("0.0.0.0"));
        assert!(is_unroutable_advertise("::"));
        assert!(is_unroutable_advertise("[::]"));
        assert!(is_unroutable_advertise(""));
        assert!(is_unroutable_advertise("   "));
    }

    #[test]
    fn a_routable_address_is_accepted() {
        assert!(!is_unroutable_advertise("51.15.248.118"));
        assert!(!is_unroutable_advertise("10.0.0.7"));
        assert!(!is_unroutable_advertise("2001:db8::1"));
    }

    #[test]
    fn a_hostname_is_accepted_rather_than_resolved() {
        // Resolving here would let a DNS answer decide whether a node may
        // start, and the answer can differ from the one a peer gets. An
        // operator who writes a name has said what they mean.
        assert!(!is_unroutable_advertise("db1.internal.example"));
    }

    #[test]
    fn an_advertise_that_already_has_a_port_does_not_get_a_second_one() {
        // The bug, exactly. `10.0.0.1:6746:6746` does not resolve, the failure
        // was on a discarded Result, and a joining node ended up admitted by the
        // seed while knowing nobody itself.
        assert_eq!(with_port("10.0.0.1:6746", 6746), "10.0.0.1:6746");
        assert_eq!(with_port("10.0.0.1:6746", 9999), "10.0.0.1:6746");
    }

    #[test]
    fn a_bare_host_still_gets_the_port_appended() {
        assert_eq!(with_port("10.0.0.1", 6746), "10.0.0.1:6746");
        assert_eq!(with_port("db1.example.com", 6746), "db1.example.com:6746");
    }

    #[test]
    fn a_hostname_carrying_a_port_is_recognised_even_though_no_ip_parser_accepts_it() {
        assert_eq!(
            with_port("db1.example.com:6746", 9999),
            "db1.example.com:6746"
        );
    }

    #[test]
    fn a_bare_ipv6_address_is_bracketed_before_a_port_is_added() {
        // Its own colons are not a port. Appending one without brackets gives
        // `::1:6746`, which parses as a different IPv6 address entirely — a
        // failure that only appears at connect time.
        assert_eq!(with_port("2001:db8::1", 6746), "[2001:db8::1]:6746");
        assert_eq!(with_port("::1", 6746), "[::1]:6746");
    }

    #[test]
    fn a_bracketed_ipv6_address_is_handled_both_ways() {
        assert_eq!(with_port("[2001:db8::1]", 6746), "[2001:db8::1]:6746");
        assert_eq!(with_port("[2001:db8::1]:6746", 9999), "[2001:db8::1]:6746");
    }

    #[test]
    fn a_port_does_not_let_loopback_past_the_guard() {
        // The hole the fix opened and closed in the same change: with a port
        // attached the value stopped parsing as an IP, so the guard caught the
        // bare form and missed the one an operator would type after reading its
        // own error message.
        assert!(is_unroutable_advertise("127.0.0.1:6746"));
        assert!(is_unroutable_advertise("[::1]:6746"));
        assert!(is_unroutable_advertise("localhost:6746"));
        assert!(is_unroutable_advertise("0.0.0.0:6746"));
    }

    #[test]
    fn a_routable_address_passes_in_either_form() {
        assert!(!is_unroutable_advertise("10.0.0.1"));
        assert!(!is_unroutable_advertise("10.0.0.1:6746"));
        assert!(!is_unroutable_advertise("db1.example.com:6746"));
        assert!(!is_unroutable_advertise("[2001:db8::1]:6746"));
    }

    #[test]
    fn the_host_is_extracted_without_brackets_or_port() {
        assert_eq!(advertised_host("10.0.0.1:6746"), "10.0.0.1");
        assert_eq!(advertised_host("[2001:db8::1]:6746"), "2001:db8::1");
        assert_eq!(advertised_host("2001:db8::1"), "2001:db8::1");
        assert_eq!(advertised_host("db1.example.com"), "db1.example.com");
    }

    fn peers(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_single_host_cluster_on_loopback_is_allowed() {
        // How anyone first tries this. Two ports on one machine are two real
        // endpoints, and an earlier version of the guard refused it — making the
        // first thing a person does impossible.
        assert!(check_advertise("127.0.0.1:6747", &peers(&["127.0.0.1:6746"])).is_ok());
        assert!(check_advertise("[::1]:6747", &peers(&["[::1]:6746"])).is_ok());
    }

    #[test]
    fn loopback_is_refused_the_moment_one_peer_is_elsewhere() {
        // The case the guard exists for: that peer reads 127.0.0.1 as itself.
        let error = check_advertise("127.0.0.1:6746", &peers(&["10.0.0.2:6746"])).unwrap_err();
        assert!(error.contains("its own loopback"), "{error}");
        assert!(error.contains("10.0.0.2:6746"), "{error}");
    }

    #[test]
    fn a_mixed_peer_list_is_refused_rather_than_averaged() {
        // One remote peer is enough. A guard that allowed this because *most*
        // peers were local would fail exactly where it matters.
        assert!(check_advertise(
            "127.0.0.1:6746",
            &peers(&["127.0.0.1:6747", "10.0.0.2:6746"])
        )
        .is_err());
    }

    #[test]
    fn the_unspecified_address_is_refused_even_on_one_host() {
        // Not a routing question: no peer can dial "every interface", however
        // close it is.
        let error = check_advertise("0.0.0.0:6746", &peers(&["127.0.0.1:6747"])).unwrap_err();
        assert!(error.contains("bind address"), "{error}");
        assert!(check_advertise("::", &peers(&["127.0.0.1:6747"])).is_err());
    }

    #[test]
    fn no_peers_means_nothing_to_check() {
        // A single node advertises to nobody, so any value is harmless.
        assert!(check_advertise("127.0.0.1", &[]).is_ok());
        assert!(check_advertise("0.0.0.0", &[]).is_ok());
    }

    #[test]
    fn a_routable_advertise_passes_whatever_the_peers_are() {
        assert!(check_advertise("10.0.0.1:6746", &peers(&["10.0.0.2:6746"])).is_ok());
        assert!(check_advertise("10.0.0.1:6746", &peers(&["127.0.0.1:6747"])).is_ok());
    }
}
