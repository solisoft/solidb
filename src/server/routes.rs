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
use crate::server::cursor_store::CursorStore;
use crate::storage::StorageEngine;
use crate::scripting::ScriptStats;

pub fn create_router(
    storage: StorageEngine,
    cluster_manager: Option<Arc<crate::cluster::manager::ClusterManager>>,
    replication_log: Option<Arc<crate::sync::log::SyncLog>>,
    shard_coordinator: Option<Arc<crate::sharding::ShardCoordinator>>,
    queue_worker: Option<Arc<crate::queue::QueueWorker>>,
    script_stats: Arc<ScriptStats>,
    _api_port: u16
) -> Router {
    // Initialize Auth (create default admin if needed)
    tracing::info!("Initializing authentication...");

    // Auth init needs to know if we are in a cluster to maybe skip default admin creation
    // The previous logic checked cluster_config.peers.
    // New ClusterManager handles joining.
    // For now we pass replication_log to auth init.
    if let Err(e) = crate::server::auth::AuthService::init(&storage, replication_log.as_deref()) {
        tracing::error!("Failed to initialize authentication: {}", e);
    } else {
        tracing::info!("Authentication initialized successfully");
    }

    // Initialize system collections in _system db
    if let Ok(db) = storage.get_database("_system") {
         // Initialize _scripts collection
         if db.get_collection("_scripts").is_err() {
             tracing::info!("Initializing _scripts collection...");
             if let Err(e) = db.create_collection("_scripts".to_string(), None) {
                 tracing::warn!("Failed to create _scripts collection (might exist): {}", e);
             }
         }

         // Initialize _roles collection for RBAC
         let roles_coll_exists = db.get_collection("_roles").is_ok();
         if !roles_coll_exists {
             tracing::info!("Initializing _roles collection...");
             if let Err(e) = db.create_collection("_roles".to_string(), None) {
                 tracing::warn!("Failed to create _roles collection (might exist): {}", e);
             }
         }
         // Seed builtin roles if missing
         if let Ok(roles_coll) = db.get_collection("_roles") {
             use crate::server::authorization::Role;
             for role in Role::builtin_roles() {
                 // Check if role already exists
                 if roles_coll.get(&role.name).is_err() {
                     if let Ok(role_json) = serde_json::to_value(&role) {
                         if let Err(e) = roles_coll.insert(role_json) {
                             tracing::warn!("Failed to insert builtin role {}: {}", role.name, e);
                         } else {
                             tracing::info!("Created builtin role: {}", role.name);
                         }
                     }
                 }
             }
         }

         // Initialize _user_roles collection for RBAC
         if db.get_collection("_user_roles").is_err() {
             tracing::info!("Initializing _user_roles collection...");
             if let Err(e) = db.create_collection("_user_roles".to_string(), None) {
                 tracing::warn!("Failed to create _user_roles collection (might exist): {}", e);
             }
         }
    }

    // Use the shared shard coordinator passed in from main.rs
    // This ensures all parts of the application share the same shard table cache

    // Initialize permission cache with builtin roles
    let permission_cache = crate::server::permission_cache::PermissionCache::new();
    permission_cache.initialize_builtin_roles();

    let state = AppState {
        storage: Arc::new(storage),
        cursor_store: CursorStore::new(Duration::from_secs(300)),
        cluster_manager,
        replication_log,
        shard_coordinator,
        queue_worker,
        startup_time: std::time::Instant::now(),
        request_counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        system_monitor: Arc::new(std::sync::Mutex::new(sysinfo::System::new())),
        script_stats,
        permission_cache,
        repl_sessions: crate::server::repl_session::ReplSessionStore::new(),
    };


    // Protected API routes
    let api_routes = Router::new()
        // Database routes
        .route("/_api/database", post(create_database))
        .route("/_api/databases", get(list_databases))
        .route("/_api/database/{name}", delete(delete_database))
        // Collection routes
        .route("/_api/database/{db}/collection", post(create_collection))
        .route("/_api/database/{db}/collection", get(list_collections))
        .route(
            "/_api/database/{db}/collection/{name}",
            delete(delete_collection),
        )
        .route(
            "/_api/database/{db}/collection/{name}/truncate",
            put(truncate_collection),
        )
        .route(
            "/_api/database/{db}/collection/{name}/compact",
            put(compact_collection),
        )
        .route(
            "/_api/database/{db}/collection/{name}/prune",
            post(prune_collection),
        )
        .route(
            "/_api/database/{db}/collection/{name}/recount",
            put(recount_collection),
        )
        .route(
            "/_api/database/{db}/collection/{name}/repair",
            post(repair_collection),
        )
        .route(
            "/_api/database/{db}/collection/{name}/stats",
            get(get_collection_stats),
        )
        .route(
            "/_api/database/{db}/collection/{name}/sharding",
            get(get_sharding_details),
        )
        .route(
            "/_api/database/{db}/collection/{name}/count",
            get(get_collection_count),
        )
        .route(
            "/_api/database/{db}/collection/{name}/properties",
            put(update_collection_properties),
        )
        .route(
            "/_api/database/{db}/collection/{name}/export",
            get(export_collection),
        )

        .route(
            "/_api/database/{db}/collection/{name}/import",
            post(import_collection).layer(DefaultBodyLimit::max(500 * 1024 * 1024)),
        )
        .route(
            "/_api/database/{db}/collection/{name}/_copy_shard",
            post(copy_shard_data),
        )
        // Document routes
        .route(
            "/_api/database/{db}/document/{collection}",
            post(insert_document),
        )
        .route(
            "/_api/database/{db}/document/{collection}/_batch",
            post(insert_documents_batch),
        )
        .route(
            "/_api/database/{db}/document/{collection}/_replica",
            post(insert_documents_replica),
        )
        .route(
            "/_api/database/{db}/document/{collection}/_verify",
            post(verify_documents_exist),
        )
        .route(
            "/_api/database/{db}/document/{collection}/{key}",
            get(get_document),
        )
        .route(
            "/_api/database/{db}/document/{collection}/{key}",
            put(update_document),
        )
        .route(
            "/_api/database/{db}/document/{collection}/{key}",
            delete(delete_document),
        )
        // Blob routes
        .route("/_api/blob/{db}/{collection}", post(upload_blob).layer(DefaultBodyLimit::max(500 * 1024 * 1024)))
        .route("/_api/blob/{db}/{collection}/{key}", get(download_blob))
        // Query routes
        .route("/_api/database/{db}/cursor", post(execute_query))
        .route("/_api/cursor/{id}", put(get_next_batch))
        .route("/_api/cursor/{id}", delete(delete_cursor))
        .route("/_api/database/{db}/explain", post(explain_query))
        // Index routes
        .route("/_api/database/{db}/index/{collection}", post(create_index))
        .route("/_api/database/{db}/index/{collection}", get(list_indexes))
        .route(
            "/_api/database/{db}/index/{collection}/rebuild",
            put(rebuild_indexes),
        )
        .route(
            "/_api/database/{db}/index/{collection}/{name}",
            delete(delete_index),
        )
        // Geo index routes
        .route("/_api/database/{db}/geo/{collection}", post(create_geo_index))
        .route("/_api/database/{db}/geo/{collection}", get(list_geo_indexes))
        .route(
            "/_api/database/{db}/geo/{collection}/{name}",
            delete(delete_geo_index),
        )
        .route(
            "/_api/database/{db}/geo/{collection}/{field}/near",
            post(geo_near),
        )
        .route(
            "/_api/database/{db}/geo/{collection}/{field}/within",
            post(geo_within),
        )
        // TTL index routes
        .route("/_api/database/{db}/ttl/{collection}", post(create_ttl_index))
        .route("/_api/database/{db}/ttl/{collection}", get(list_ttl_indexes))
        .route(
            "/_api/database/{db}/ttl/{collection}/{name}",
            delete(delete_ttl_index),
        )
        // Schema routes
        .route(
            "/_api/database/{db}/collection/{name}/schema",
            post(super::handlers::set_collection_schema),
        )
        .route(
            "/_api/database/{db}/collection/{name}/schema",
            get(super::handlers::get_collection_schema),
        )
        .route(
            "/_api/database/{db}/collection/{name}/schema",
            delete(super::handlers::delete_collection_schema),
        )
        // Transaction routes
        .route("/_api/database/{db}/transaction/begin", post(super::transaction_handlers::begin_transaction))
        .route("/_api/database/{db}/transaction/{tx_id}/commit", post(super::transaction_handlers::commit_transaction))
        .route("/_api/database/{db}/transaction/{tx_id}/rollback", post(super::transaction_handlers::rollback_transaction))
        // Transaction operations (missing routes added)
        .route("/_api/database/{db}/transaction/{tx_id}/document/{collection}", post(super::transaction_handlers::insert_document_tx))
        .route("/_api/database/{db}/transaction/{tx_id}/document/{collection}/{key}", put(super::transaction_handlers::update_document_tx))
        .route("/_api/database/{db}/transaction/{tx_id}/document/{collection}/{key}", delete(super::transaction_handlers::delete_document_tx))
        .route("/_api/database/{db}/transaction/{tx_id}/query", post(super::transaction_handlers::execute_transactional_sdbql))
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
        .route("/_api/auth/api-keys/{key_id}", delete(delete_api_key_handler))
        // Role management (RBAC)
        .route("/_api/auth/roles", get(super::role_handlers::list_roles))
        .route("/_api/auth/roles", post(super::role_handlers::create_role))
        .route("/_api/auth/roles/{name}", get(super::role_handlers::get_role))
        .route("/_api/auth/roles/{name}", put(super::role_handlers::update_role))
        .route("/_api/auth/roles/{name}", delete(super::role_handlers::delete_role))
        // User management
        .route("/_api/auth/users", get(super::role_handlers::list_users))
        .route("/_api/auth/users", post(super::role_handlers::create_user))
        .route("/_api/auth/users/{username}", delete(super::role_handlers::delete_user))
        // User role management
        .route("/_api/auth/users/{username}/roles", get(super::role_handlers::get_user_roles))
        .route("/_api/auth/users/{username}/roles", post(super::role_handlers::assign_role))
        .route("/_api/auth/users/{username}/roles/{role}", delete(super::role_handlers::revoke_role))
        // Self-service endpoints
        .route("/_api/auth/me", get(super::role_handlers::get_current_user))
        .route("/_api/auth/me/permissions", get(super::role_handlers::get_my_permissions))
        // Queue Management
        .route("/_api/database/{db}/queues", get(super::queue_handlers::list_queues_handler))
        .route("/_api/database/{db}/queues/{name}/jobs", get(super::queue_handlers::list_jobs_handler))
        .route("/_api/database/{db}/queues/{name}/enqueue", post(super::queue_handlers::enqueue_job_handler))
        .route("/_api/database/{db}/queues/jobs/{id}", delete(super::queue_handlers::cancel_job_handler))
        // Cron Job Management
        .route("/_api/database/{db}/cron", get(super::queue_handlers::list_cron_jobs_handler))
        .route("/_api/database/{db}/cron", post(super::queue_handlers::create_cron_job_handler))
        .route("/_api/database/{db}/cron/{id}", put(super::queue_handlers::update_cron_job_handler))
        .route("/_api/database/{db}/cron/{id}", delete(super::queue_handlers::delete_cron_job_handler))
        // Script management routes
        .route("/_api/database/{db}/scripts", post(super::script_handlers::create_script_handler))
        .route("/_api/database/{db}/scripts", get(super::script_handlers::list_scripts_handler))
        .route("/_api/database/{db}/scripts/{script_id}", get(super::script_handlers::get_script_handler))
        .route("/_api/database/{db}/scripts/{script_id}", put(super::script_handlers::update_script_handler))
        .route("/_api/database/{db}/scripts/{script_id}", delete(super::script_handlers::delete_script_handler))
        .route("/_api/scripts/stats", get(super::script_handlers::get_script_stats_handler))
        // REPL endpoint for interactive Lua execution
        .route("/_api/database/{db}/repl", post(super::script_handlers::repl_eval_handler))
        .route("/_api/monitoring/ws", get(super::handlers::monitor_ws_handler))
        // Live Query Token (short-lived token for WebSocket connections)
        .route("/_api/livequery/token", get(livequery_token_handler))
        .route_layer(axum::middleware::from_fn_with_state(state.clone(), crate::server::auth::auth_middleware));

    // Combine with public routes
    Router::new()
        .route("/auth/login", post(login_handler))
        // Health check endpoint for cluster node monitoring (no auth required)
        .route("/_api/health", get(health_check_handler))
        // Internal cluster endpoints (use cluster secret, no user auth)
        .route("/_api/cluster/cleanup", post(cluster_cleanup))
        .route("/_api/cluster/reshard", post(cluster_reshard))
        // Internal Blob Replication endpoint
        .route(
            "/_internal/blob/replicate/{db}/{collection}/{key}",
            post(crate::sync::blob_replication::receive_blob_replication).layer(DefaultBodyLimit::max(500 * 1024 * 1024))
        )
        // Internal Blob Chunk fetch endpoint
        .route(
            "/_internal/blob/replicate/{db}/{collection}/{key}/chunk/{chunk_idx}",
            get(crate::sync::blob_replication::get_blob_chunk)
        )
        // Internal Blob Upload endpoint (for shard coordinator forwarding)
        .route(
            "/_internal/blob/upload/{db}/{collection}",
            post(crate::sync::blob_replication::receive_blob_upload).layer(DefaultBodyLimit::max(500 * 1024 * 1024))
        )
        // WebSocket route (outside auth middleware - uses token in query param)
        .route("/_api/cluster/status/ws", get(cluster_status_ws))
        .route("/_api/ws/changefeed", get(ws_changefeed_handler))
        // Custom Lua API endpoints (public, script handles own auth if needed)
        .route("/api/custom/{*path}", get(super::script_handlers::execute_script_handler))
        .route("/api/custom/{*path}", post(super::script_handlers::execute_script_handler))
        .route("/api/custom/{*path}", put(super::script_handlers::execute_script_handler))
        .route("/api/custom/{*path}", delete(super::script_handlers::execute_script_handler))
        .merge(api_routes)
        .with_state(state)
        // Global request body limit: 10MB default (import/blob have 500MB override)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
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
