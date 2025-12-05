use axum::http::Method;
use axum::{
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
    // Initialize ShardCoordinator if in cluster mode
    let shard_coordinator = if let Some(config) = storage.cluster_config() {
        if config.is_cluster_mode() {
            // Use peers list as cluster definition
            // TODO: Robustly determine own index and ensure peers are HTTP API addresses
            let node_addresses = config.peers.clone();
            let node_index = 0; // Default to 0 for now
            
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
    };

    Router::new()
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
        .with_state(state)
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
