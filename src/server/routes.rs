use axum::http::Method;
use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post, put},
    Router,
};
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use super::handlers::*;
use crate::cluster::ReplicationService;
use crate::server::cursor_store::CursorStore;
use crate::storage::StorageEngine;

pub fn create_router(storage: StorageEngine, replication: Option<ReplicationService>) -> Router {
    // Initialize Auth (create default admin if needed)
    tracing::info!("Initializing authentication...");
    if let Err(e) = crate::server::auth::AuthService::init(&storage, replication.as_ref()) {
        tracing::error!("Failed to initialize authentication: {}", e);
    } else {
        tracing::info!("Authentication initialized successfully");
    }

    // Initialize ShardCoordinator if in cluster mode
    let shard_coordinator = if let Some(config) = storage.cluster_config() {
        if config.is_cluster_mode() {
            // Get this node's API address (http port = replication port - 1)
            let my_api_port = config.replication_port - 1;
            let my_api_addr = format!("localhost:{}", my_api_port);
            
            // Convert replication addresses (port 6746) to HTTP API addresses (port 6745)
            // The peers list contains replication addresses, but ShardCoordinator needs API addresses
            let mut node_addresses: Vec<String> = config.peers.iter().map(|peer| {
                // Parse the peer address and convert to API port
                // Replication ports are typically API port + 1
                if let Some(port_start) = peer.rfind(':') {
                    let host = &peer[..port_start];
                    if let Ok(repl_port) = peer[port_start+1..].parse::<u16>() {
                        let api_port = repl_port - 1; // API port is typically one less
                        return format!("{}:{}", host, api_port);
                    }
                }
                peer.clone() // Fallback: use as-is
            }).collect();
            
            // Add self to node_addresses if not already present
            if !node_addresses.contains(&my_api_addr) {
                node_addresses.insert(0, my_api_addr.clone()); // Self is always first
            }
            
            // Sort addresses for consistent ordering across all nodes
            node_addresses.sort();
            
            // Find my index in the sorted list
            let node_index = node_addresses.iter()
                .position(|addr| addr == &my_api_addr)
                .unwrap_or(0);
            
            tracing::info!("ShardCoordinator initialized: my_addr={}, node_index={}, nodes: {:?}", 
                my_api_addr, node_index, node_addresses);
            
            let coordinator = crate::sharding::ShardCoordinator::with_health_tracking(
                Arc::new(storage.clone()),
                node_index,
                node_addresses,
                3, // failure threshold
            );
            
            let coord_arc = Arc::new(coordinator);
            coord_arc.clone().start_background_tasks();
            
            Some((*coord_arc).clone())
        } else {
            None
        }
    } else {
        None
    };

    let state = AppState {
        storage: Arc::new(storage),
        cursor_store: CursorStore::new(Duration::from_secs(300)),
        replication,
        shard_coordinator,
        startup_time: std::time::Instant::now(),
        request_counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
    };

    // Protected API routes
    let api_routes = Router::new()
        // Database routes
        .route("/_api/database", post(create_database))
        .route("/_api/databases", get(list_databases))
        .route("/_api/database/:name", delete(delete_database))
        // Collection routes
        .route("/_api/database/:db/collection", post(create_collection))
        .route("/_api/database/:db/collection", get(list_collections))
        .route(
            "/_api/database/:db/collection/:name",
            delete(delete_collection),
        )
        .route(
            "/_api/database/:db/collection/:name/truncate",
            put(truncate_collection),
        )
        .route(
            "/_api/database/:db/collection/:name/compact",
            put(compact_collection),
        )
        .route(
            "/_api/database/:db/collection/:name/stats",
            get(get_collection_stats),
        )
        .route(
            "/_api/database/:db/collection/:name/properties",
            put(update_collection_properties),
        )
        .route(
            "/_api/database/:db/collection/:name/export",
            get(export_collection),
        )
        .route(
            "/_api/database/:db/collection/:name/import",
            post(import_collection).layer(DefaultBodyLimit::max(500 * 1024 * 1024)),
        )
        // Document routes
        .route(
            "/_api/database/:db/document/:collection",
            post(insert_document),
        )
        .route(
            "/_api/database/:db/document/:collection/:key",
            get(get_document),
        )
        .route(
            "/_api/database/:db/document/:collection/:key",
            put(update_document),
        )
        .route(
            "/_api/database/:db/document/:collection/:key",
            delete(delete_document),
        )
        // Blob routes
        .route("/_api/blob/:db/:collection", post(upload_blob).layer(DefaultBodyLimit::max(500 * 1024 * 1024)))
        .route("/_api/blob/:db/:collection/:key", get(download_blob))
        // Query routes
        .route("/_api/database/:db/cursor", post(execute_query))
        .route("/_api/cursor/:id", put(get_next_batch))
        .route("/_api/cursor/:id", delete(delete_cursor))
        .route("/_api/database/:db/explain", post(explain_query))
        // Index routes
        .route("/_api/database/:db/index/:collection", post(create_index))
        .route("/_api/database/:db/index/:collection", get(list_indexes))
        .route(
            "/_api/database/:db/index/:collection/rebuild",
            put(rebuild_indexes),
        )
        .route(
            "/_api/database/:db/index/:collection/:name",
            delete(delete_index),
        )
        // Geo index routes
        .route("/_api/database/:db/geo/:collection", post(create_geo_index))
        .route("/_api/database/:db/geo/:collection", get(list_geo_indexes))
        .route(
            "/_api/database/:db/geo/:collection/:name",
            delete(delete_geo_index),
        )
        .route(
            "/_api/database/:db/geo/:collection/:field/near",
            post(geo_near),
        )
        .route(
            "/_api/database/:db/geo/:collection/:field/within",
            post(geo_within),
        )
        // Transaction routes
        .route("/_api/database/:db/transaction/begin", post(super::transaction_handlers::begin_transaction))
        .route("/_api/database/:db/transaction/:tx_id/commit", post(super::transaction_handlers::commit_transaction))
        .route("/_api/database/:db/transaction/:tx_id/rollback", post(super::transaction_handlers::rollback_transaction))
        // Cluster routes
        .route("/_api/cluster/status", get(cluster_status))
        .route("/_api/cluster/info", get(cluster_info))
        .route("/_api/cluster/remove-node", post(cluster_remove_node))
        .route("/_api/cluster/rebalance", post(cluster_rebalance))
// WebSocket routes (moved to public router)
        // .route("/_api/ws/changefeed", get(ws_changefeed_handler))
        // Auth management
        .route("/_api/auth/password", put(change_password_handler))
        .route("/_api/auth/api-keys", post(create_api_key_handler))
        .route("/_api/auth/api-keys", get(list_api_keys_handler))
        .route("/_api/auth/api-keys/:key_id", delete(delete_api_key_handler))
        // Script management routes
        .route("/_api/database/:db/scripts", post(super::script_handlers::create_script_handler))
        .route("/_api/database/:db/scripts", get(super::script_handlers::list_scripts_handler))
        .route("/_api/database/:db/scripts/:script_id", get(super::script_handlers::get_script_handler))
        .route("/_api/database/:db/scripts/:script_id", put(super::script_handlers::update_script_handler))
        .route("/_api/database/:db/scripts/:script_id", delete(super::script_handlers::delete_script_handler))
        // Apply authentication middleware
        .route_layer(axum::middleware::from_fn_with_state(state.clone(), crate::server::auth::auth_middleware));

    // Combine with public routes
    Router::new()
        .route("/auth/login", post(login_handler))
        // WebSocket route (outside auth middleware - uses token in query param)
        .route("/_api/cluster/status/ws", get(cluster_status_ws))
        .route("/_api/ws/changefeed", get(ws_changefeed_handler))
        // Custom Lua API endpoints (public, script handles own auth if needed)
        .route("/api/custom/*path", get(super::script_handlers::execute_script_handler))
        .route("/api/custom/*path", post(super::script_handlers::execute_script_handler))
        .route("/api/custom/*path", put(super::script_handlers::execute_script_handler))
        .route("/api/custom/*path", delete(super::script_handlers::execute_script_handler))
        .merge(api_routes)
        .with_state(state)
        // Global request body limit: 10MB default (import/blob have 500MB override)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(
                    "http://localhost:8080"
                        .parse::<axum::http::HeaderValue>()
                        .unwrap(),
                )
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::DELETE,
                    Method::OPTIONS,
                ])
                .allow_headers(Any),
        )
}
