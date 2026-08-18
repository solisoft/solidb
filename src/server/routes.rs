use axum::http::Method;
use axum::{
    body::Body,
    extract::DefaultBodyLimit,
    http::Request,
    middleware::Next,
    response::Response,
    routing::{delete, get, post, put},
    Router,
};
use std::sync::Arc;
use std::time::Duration;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowHeaders, CorsLayer};
use tower_http::trace::TraceLayer;

/// Parse CORS allowed origins from environment variable.
/// Defaults to restrictive (no cross-origin) if not set.
/// Security: Empty string or wildcard in production is discouraged.
fn get_cors_allowed_origins() -> Vec<String> {
    let env_value = std::env::var("SOLIDB_CORS_ALLOWED_ORIGINS").unwrap_or_default();
    if env_value.is_empty() {
        return vec![];
    }
    if env_value == "*" || env_value == "*:*" {
        tracing::warn!(
            "CORS configured with wildcard '*'. This allows ANY origin. \
            Set SOLIDB_CORS_ALLOWED_ORIGINS to specific origins in production."
        );
        return vec!["*".to_string()];
    }
    env_value
        .split(',')
        .filter_map(|origin| {
            let origin = origin.trim();
            if origin.is_empty() || !is_valid_origin(origin) {
                if !origin.is_empty() {
                    tracing::warn!("Invalid CORS origin '{}' - skipping", origin);
                }
                return None;
            }
            Some(origin.to_string())
        })
        .collect()
}

/// An origin must look like `scheme://host[:port]` with no path/query/fragment.
/// `axum::http::Uri::parse` accepts paths and bare strings, which CORS shouldn't.
fn is_valid_origin(s: &str) -> bool {
    let parsed = match url::Url::parse(s) {
        Ok(u) => u,
        Err(_) => return false,
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    if parsed.host_str().is_none() {
        return false;
    }
    // Must be exactly an origin: empty path, no query, no fragment.
    if parsed.path() != "/" && !parsed.path().is_empty() {
        return false;
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return false;
    }
    // url::Url stringifies origins with a trailing "/" — accept either form
    // but reject inputs that contained extra structure beyond host[:port].
    let canonical_no_slash = format!(
        "{}://{}{}",
        parsed.scheme(),
        parsed.host_str().unwrap(),
        parsed.port().map(|p| format!(":{}", p)).unwrap_or_default()
    );
    s == canonical_no_slash || s == format!("{}/", canonical_no_slash)
}

/// Middleware to count incoming requests
async fn request_counter_middleware(
    axum::extract::State(state): axum::extract::State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    state
        .request_counter
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    next.run(request).await
}

/// Middleware to extract and store trace context from incoming requests
async fn trace_context_middleware(request: Request<Body>, next: Next) -> Response {
    use crate::observability::propagation::{extract_trace_context, TRACEPARENT_HEADER};

    let trace_ctx = extract_trace_context(&request);

    let mut req = request;
    if let Some(ctx) = trace_ctx {
        req.extensions_mut().insert(ctx);
    }

    let span = tracing::info_span!(
        "http_request",
        http.method = %req.method(),
        http.url = %req.uri(),
        traceparent = req.headers().get(TRACEPARENT_HEADER).map(|h| h.to_str().unwrap_or("")).unwrap_or("")
    );
    span.follows_from(tracing::Span::current());

    next.run(req).await
}

use super::handlers::*;
use super::nl_handlers;
use crate::scripting::engine::{LuaPool, ScriptCache, ScriptIndex};
use crate::scripting::ScriptStats;
use crate::server::cursor_store::CursorStore;
use crate::server::upload_session::UploadSessionStore;
use crate::storage::StorageEngine;

#[allow(clippy::too_many_arguments)]
pub fn create_router(
    storage: StorageEngine,
    cluster_manager: Option<Arc<crate::cluster::manager::ClusterManager>>,
    replication_log: Option<Arc<crate::sync::log::SyncLog>>,
    shard_coordinator: Option<Arc<crate::sharding::ShardCoordinator>>,
    queue_worker: Option<Arc<crate::queue::QueueWorker>>,
    script_stats: Arc<ScriptStats>,
    stream_manager: Option<Arc<crate::stream::StreamManager>>,
    blob_rebalance_worker: Option<Arc<crate::sharding::BlobRebalanceWorker>>,
    _api_port: u16,
) -> Router {
    // Initialize Auth (create default admin if needed)
    tracing::info!("Initializing authentication...");

    // Auth init needs to know if we are in a cluster to maybe skip default admin creation
    // The previous logic checked cluster_config.peers.
    // New ClusterManager handles joining.
    // For now we pass replication_log to auth init.
    // Security: data_dir is passed to save admin password to a file instead of logging it
    if let Err(e) = crate::server::auth::AuthService::init(
        &storage,
        replication_log.as_deref(),
        storage.data_dir(),
    ) {
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
                tracing::warn!(
                    "Failed to create _user_roles collection (might exist): {}",
                    e
                );
            }
        }
    }

    // Use the shared shard coordinator passed in from main.rs
    // This ensures all parts of the application share the same shard table cache

    // Initialize permission cache with builtin roles
    let permission_cache = crate::server::permission_cache::PermissionCache::new();
    permission_cache.initialize_builtin_roles();

    // Initialize sync session manager for offline-first client sync
    let sync_session_manager = Arc::new(crate::sync::SyncSessionManager::new());

    let lua_enabled = crate::scripting::lua_runtime_enabled();

    // Wrap storage in Arc before building the index
    let storage = Arc::new(storage);

    let (lua_pool, script_cache, script_index, service_cache) = if lua_enabled {
        let lua_pool = Arc::new(LuaPool::with_default_size());
        tracing::info!(
            "Lua VM pool initialized with {} states",
            lua_pool.stats().size
        );

        let script_cache = Arc::new(ScriptCache::with_default_size());
        let script_index = Arc::new(ScriptIndex::new());
        script_index.rebuild(&storage);
        let index_stats = script_index.stats();
        tracing::info!(
            "Script index built: {} exact paths, {} pattern paths",
            index_stats.exact_entries,
            index_stats.pattern_entries
        );

        let service_cache = Arc::new(crate::server::service_cache::ServiceCache::new(5));
        {
            let mut scripts_warmed = 0u32;
            let mut services_warmed = 0u32;
            let temp_lua = mlua::Lua::new();

            for db_name in storage.list_databases() {
                if let Ok(db) = storage.get_database(&db_name) {
                    if let Ok(collection) = db.get_collection("_scripts") {
                        for doc in collection.scan(None) {
                            if let Ok(script) =
                                serde_json::from_value::<crate::scripting::Script>(doc.to_value())
                            {
                                script_cache.get_or_analyze_needs(&script.key, &script.code);
                                let _ = script_cache.get_or_compile(
                                    &script.key,
                                    &script.code,
                                    |code| {
                                        let chunk = temp_lua.load(code);
                                        let func = chunk.into_function()?;
                                        Ok(func.dump(false))
                                    },
                                );
                                scripts_warmed += 1;
                            }
                        }
                    }
                    if let Ok(collection) = db.get_collection("_services") {
                        for doc in collection.scan(None) {
                            if let Ok(service) =
                                serde_json::from_value::<crate::scripting::Service>(doc.to_value())
                            {
                                let key = service.key.clone();
                                service_cache.insert(&db_name, &key, service);
                                services_warmed += 1;
                            }
                        }
                    }
                }
            }
            tracing::info!(
                "Caches pre-warmed: {} scripts (needs + bytecode), {} services",
                scripts_warmed,
                services_warmed
            );
        }

        (Some(lua_pool), script_cache, script_index, service_cache)
    } else {
        tracing::warn!(
            "Lua is disabled (--no-lua / SOLIDB_NO_LUA). Script execution, \
             the Lua REPL, and service endpoints are off; the VM pool was not created."
        );
        (
            None,
            Arc::new(ScriptCache::new(0)),
            Arc::new(ScriptIndex::new()),
            Arc::new(crate::server::service_cache::ServiceCache::new(0)),
        )
    };

    let cursor_store = CursorStore::new(Duration::from_secs(300));
    cursor_store.spawn_cleanup_task();

    let upload_session_store = UploadSessionStore::new(Duration::from_secs(24 * 60 * 60));
    {
        let storage_clone = storage.clone();
        upload_session_store.spawn_cleanup_task(move |db_name, coll_name, upload_id| {
            tracing::info!(
                "Cleaning up expired upload session {} for {}/{}",
                upload_id,
                db_name,
                coll_name
            );
            if let Ok(db) = storage_clone.get_database(db_name) {
                if let Ok(coll) = db.get_collection(coll_name) {
                    if let Err(e) = coll.delete_upload_chunks(upload_id) {
                        tracing::warn!(
                            "Failed to clean up temp chunks for upload {}: {}",
                            upload_id,
                            e
                        );
                    }
                }
            }
        });
    }

    let state = AppState {
        storage,
        cursor_store,
        cluster_manager,
        replication_log,
        shard_coordinator,
        queue_worker,
        startup_time: std::time::Instant::now(),
        request_counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        query_counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        write_counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),

        system_monitor: Arc::new(std::sync::Mutex::new(sysinfo::System::new())),
        script_stats,
        stream_manager,
        permission_cache,
        repl_sessions: crate::server::repl_session::ReplSessionStore::new(),
        channel_manager: Arc::new(crate::scripting::ChannelManager::new()),
        sync_session_manager: Some(sync_session_manager),
        lua_pool,
        script_cache,
        script_index,
        service_cache,
        blob_rebalance_worker,
        upload_session_store,
    };

    // Protected API routes
    let api_routes = Router::new()
        // Database routes
        .route("/_api/database", post(create_database))
        .route("/_api/databases", get(list_databases))
        .route("/_api/database/{name}", delete(delete_database))
        // Physical backup (whole instance). Admin is enforced in the handler:
        // this route has no {db} segment, so the path-based authz middleware
        // cannot scope it to a database.
        .route("/_api/backup", post(create_backup))
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
        .route(
            "/_api/blob/{db}/{collection}",
            post(upload_blob).layer(DefaultBodyLimit::max(500 * 1024 * 1024)),
        )
        .route("/_api/blob/{db}/{collection}/{key}", get(download_blob))
        // Resumable blob upload routes
        .route(
            "/_api/blob/{db}/{collection}/upload",
            post(create_upload_session),
        )
        .route(
            "/_api/blob/{db}/{collection}/upload/{upload_id}/{chunk_index}",
            post(upload_chunk).layer(DefaultBodyLimit::max(6 * 1024 * 1024)),
        )
        .route(
            "/_api/blob/{db}/{collection}/upload/{upload_id}/complete",
            post(complete_upload),
        )
        .route(
            "/_api/blob/{db}/{collection}/upload/{upload_id}/status",
            get(get_upload_status),
        )
        .route(
            "/_api/blob/{db}/{collection}/upload/{upload_id}/abort",
            delete(abort_upload),
        )
        // Query routes
        .route("/_api/database/{db}/cursor", post(execute_query))
        .route("/_api/cursor/{id}", put(get_next_batch))
        .route("/_api/cursor/{id}", delete(delete_cursor))
        .route("/_api/database/{db}/explain", post(explain_query))
        // Natural language query routes
        .route("/_api/database/{db}/nl", post(nl_handlers::nl_query))
        .route(
            "/_api/database/{db}/nl/feedback",
            post(nl_handlers::nl_feedback),
        )
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
        .route(
            "/_api/database/{db}/geo/{collection}",
            post(create_geo_index),
        )
        .route(
            "/_api/database/{db}/geo/{collection}",
            get(list_geo_indexes),
        )
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
        // Vector index routes
        .route(
            "/_api/database/{db}/vector/{collection}",
            post(create_vector_index),
        )
        .route(
            "/_api/database/{db}/vector/{collection}",
            get(list_vector_indexes),
        )
        .route(
            "/_api/database/{db}/vector/{collection}/{name}",
            delete(delete_vector_index),
        )
        .route(
            "/_api/database/{db}/vector/{collection}/{index}/search",
            post(vector_search),
        )
        .route(
            "/_api/database/{db}/vector/{collection}/{index}/quantize",
            post(quantize_vector_index),
        )
        .route(
            "/_api/database/{db}/vector/{collection}/{index}/dequantize",
            post(dequantize_vector_index),
        )
        // Hybrid search route (vector + fulltext)
        .route(
            "/_api/database/{db}/hybrid/{collection}/search",
            post(hybrid_search),
        )
        // Graph RAG: local expansion + global community retrieval
        .route("/_api/database/{db}/graph/neighbors", post(graph_neighbors))
        .route("/_api/database/{db}/graph/rag", post(graph_rag_search))
        .route(
            "/_api/database/{db}/graph/community/search",
            post(community_search),
        )
        // Global GraphRAG: community detection build + status + listing
        .route(
            "/_api/database/{db}/graph/community/build",
            post(build_communities),
        )
        .route(
            "/_api/database/{db}/graph/community/build/{request_id}",
            get(build_status),
        )
        .route(
            "/_api/database/{db}/graph/communities",
            get(list_communities),
        )
        // TTL index routes
        .route(
            "/_api/database/{db}/ttl/{collection}",
            post(create_ttl_index),
        )
        .route(
            "/_api/database/{db}/ttl/{collection}",
            get(list_ttl_indexes),
        )
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
        // Env var routes
        .route(
            "/_api/database/{db}/env",
            get(super::env_handlers::list_env_vars_handler),
        )
        .route(
            "/_api/database/{db}/env/{key}",
            put(super::env_handlers::set_env_var_handler),
        )
        .route(
            "/_api/database/{db}/env/{key}",
            delete(super::env_handlers::delete_env_var_handler),
        )
        // Columnar collection routes
        .route(
            "/_api/database/{db}/columnar",
            post(super::columnar_handlers::create_columnar_handler),
        )
        .route(
            "/_api/database/{db}/columnar",
            get(super::columnar_handlers::list_columnar_handler),
        )
        .route(
            "/_api/database/{db}/columnar/{collection}",
            get(super::columnar_handlers::get_columnar_handler),
        )
        .route(
            "/_api/database/{db}/columnar/{collection}",
            delete(super::columnar_handlers::delete_columnar_handler),
        )
        .route(
            "/_api/database/{db}/columnar/{collection}/insert",
            post(super::columnar_handlers::insert_columnar_handler),
        )
        .route(
            "/_api/database/{db}/columnar/{collection}/aggregate",
            post(super::columnar_handlers::aggregate_columnar_handler),
        )
        .route(
            "/_api/database/{db}/columnar/{collection}/query",
            post(super::columnar_handlers::query_columnar_handler),
        )
        // Columnar index routes
        .route(
            "/_api/database/{db}/columnar/{collection}/index",
            post(super::columnar_handlers::create_columnar_index_handler),
        )
        .route(
            "/_api/database/{db}/columnar/{collection}/indexes",
            get(super::columnar_handlers::list_columnar_indexes_handler),
        )
        .route(
            "/_api/database/{db}/columnar/{collection}/index/{column}",
            delete(super::columnar_handlers::delete_columnar_index_handler),
        )
        // Transaction routes
        .route(
            "/_api/database/{db}/transaction/begin",
            post(super::transaction_handlers::begin_transaction),
        )
        .route(
            "/_api/database/{db}/transaction/{tx_id}/commit",
            post(super::transaction_handlers::commit_transaction),
        )
        .route(
            "/_api/database/{db}/transaction/{tx_id}/rollback",
            post(super::transaction_handlers::rollback_transaction),
        )
        // Transaction operations (missing routes added)
        .route(
            "/_api/database/{db}/transaction/{tx_id}/document/{collection}",
            post(super::transaction_handlers::insert_document_tx),
        )
        .route(
            "/_api/database/{db}/transaction/{tx_id}/document/{collection}/{key}",
            put(super::transaction_handlers::update_document_tx),
        )
        .route(
            "/_api/database/{db}/transaction/{tx_id}/document/{collection}/{key}",
            delete(super::transaction_handlers::delete_document_tx),
        )
        .route(
            "/_api/database/{db}/transaction/{tx_id}/query",
            post(super::transaction_handlers::execute_transactional_sdbql),
        )
        // Distributed transaction routes
        .route(
            "/_api/distributed/transaction/begin",
            post(super::transaction_handlers::begin_distributed_transaction),
        )
        .route(
            "/_api/distributed/transaction/{tx_id}/prepare",
            post(super::transaction_handlers::prepare_distributed_transaction),
        )
        .route(
            "/_api/distributed/transaction/{tx_id}/commit",
            post(super::transaction_handlers::commit_distributed_transaction),
        )
        .route(
            "/_api/distributed/transaction/{tx_id}/abort",
            post(super::transaction_handlers::abort_distributed_transaction),
        )
        // Distributed transaction participant routes (handlers on each shard node)
        .route(
            "/_api/distributed/participant/prepare/{tx_id}",
            post(super::transaction_handlers::participant_prepare),
        )
        .route(
            "/_api/distributed/participant/commit/{tx_id}",
            post(super::transaction_handlers::participant_commit),
        )
        .route(
            "/_api/distributed/participant/abort/{tx_id}",
            post(super::transaction_handlers::participant_abort),
        )
        // Cluster routes
        .route("/_api/cluster/status", get(cluster_status))
        .route("/_api/cluster/info", get(cluster_info))
        .route("/_api/cluster/remove-node", post(cluster_remove_node))
        .route("/_api/cluster/rebalance", post(cluster_rebalance))
        .route("/_api/cluster/blob-distribution", get(blob_distribution))
        .route("/_api/cluster/blob-rebalance", post(blob_rebalance))
        .route("/_api/cluster/sync-log/stats", get(sync_log_stats))
        .route("/_api/cluster/sync-log/prune", post(sync_log_prune))
        // WebSocket routes (moved to public router)
        // .route("/_api/ws/changefeed", get(ws_changefeed_handler))
        // Auth management
        .route("/_api/auth/password", put(change_password_handler))
        .route("/_api/auth/api-keys", post(create_api_key_handler))
        .route("/_api/auth/api-keys", get(list_api_keys_handler))
        .route(
            "/_api/auth/api-keys/{key_id}",
            delete(delete_api_key_handler),
        )
        // Role management (RBAC)
        .route("/_api/auth/roles", get(super::role_handlers::list_roles))
        .route("/_api/auth/roles", post(super::role_handlers::create_role))
        .route(
            "/_api/auth/roles/{name}",
            get(super::role_handlers::get_role),
        )
        .route(
            "/_api/auth/roles/{name}",
            put(super::role_handlers::update_role),
        )
        .route(
            "/_api/auth/roles/{name}",
            delete(super::role_handlers::delete_role),
        )
        // User management
        .route("/_api/auth/users", get(super::role_handlers::list_users))
        .route("/_api/auth/users", post(super::role_handlers::create_user))
        .route(
            "/_api/auth/users/{username}",
            delete(super::role_handlers::delete_user),
        )
        // User role management
        .route(
            "/_api/auth/users/{username}/roles",
            get(super::role_handlers::get_user_roles),
        )
        .route(
            "/_api/auth/users/{username}/roles",
            post(super::role_handlers::assign_role),
        )
        .route(
            "/_api/auth/users/{username}/roles/{role}",
            delete(super::role_handlers::revoke_role),
        )
        // Self-service endpoints
        .route("/_api/auth/me", get(super::role_handlers::get_current_user))
        .route(
            "/_api/auth/me/permissions",
            get(super::role_handlers::get_my_permissions),
        )
        // Trigger Management
        .route(
            "/_api/database/{db}/triggers",
            get(super::trigger_handlers::list_triggers_handler),
        )
        .route(
            "/_api/database/{db}/triggers",
            post(super::trigger_handlers::create_trigger_handler),
        )
        .route(
            "/_api/database/{db}/triggers/{id}",
            get(super::trigger_handlers::get_trigger_handler),
        )
        .route(
            "/_api/database/{db}/triggers/{id}",
            put(super::trigger_handlers::update_trigger_handler),
        )
        .route(
            "/_api/database/{db}/triggers/{id}",
            delete(super::trigger_handlers::delete_trigger_handler),
        )
        .route(
            "/_api/database/{db}/triggers/{id}/toggle",
            post(super::trigger_handlers::toggle_trigger_handler),
        )
        .route(
            "/_api/database/{db}/collections/{coll}/triggers",
            get(super::trigger_handlers::list_collection_triggers_handler),
        )
        // Script management routes
        .route(
            "/_api/database/{db}/scripts",
            post(super::script_handlers::create_script_handler),
        )
        .route(
            "/_api/database/{db}/scripts",
            get(super::script_handlers::list_scripts_handler),
        )
        .route(
            "/_api/database/{db}/scripts/{script_id}",
            get(super::script_handlers::get_script_handler),
        )
        .route(
            "/_api/database/{db}/scripts/{script_id}",
            put(super::script_handlers::update_script_handler),
        )
        .route(
            "/_api/database/{db}/scripts/{script_id}",
            delete(super::script_handlers::delete_script_handler),
        )
        .route(
            "/_api/scripts/stats",
            get(super::script_handlers::get_script_stats_handler),
        )
        // Service management routes
        .route(
            "/_api/database/{db}/services",
            post(super::script_handlers::create_service_handler),
        )
        .route(
            "/_api/database/{db}/services",
            get(super::script_handlers::list_services_handler),
        )
        .route(
            "/_api/database/{db}/services/{key}",
            get(super::script_handlers::get_service_handler),
        )
        .route(
            "/_api/database/{db}/services/{key}",
            put(super::script_handlers::update_service_handler),
        )
        .route(
            "/_api/database/{db}/services/{key}",
            delete(super::script_handlers::delete_service_handler),
        )
        .route(
            "/_api/database/{db}/services/{key}/openapi",
            get(super::script_handlers::get_service_openapi_handler),
        )
        // REPL endpoint for interactive Lua execution
        .route(
            "/_api/database/{db}/repl",
            post(super::script_handlers::repl_eval_handler),
        )
        .route(
            "/_api/monitoring/ws",
            get(super::handlers::monitor_ws_handler),
        )
        // AI Contribution routes
        .route(
            "/_api/database/{db}/ai/contributions",
            post(super::ai_handlers::ai_submit_contribution_handler),
        )
        .route(
            "/_api/database/{db}/ai/contributions",
            get(super::ai_handlers::ai_list_contributions_handler),
        )
        .route(
            "/_api/database/{db}/ai/contributions/{id}",
            get(super::ai_handlers::ai_get_contribution_handler),
        )
        .route(
            "/_api/database/{db}/ai/contributions/{id}/approve",
            post(super::ai_handlers::ai_approve_contribution_handler),
        )
        .route(
            "/_api/database/{db}/ai/contributions/{id}/reject",
            post(super::ai_handlers::ai_reject_contribution_handler),
        )
        .route(
            "/_api/database/{db}/ai/contributions/{id}/cancel",
            post(super::ai_handlers::ai_cancel_contribution_handler),
        )
        // AI Task routes
        .route(
            "/_api/database/{db}/ai/tasks",
            get(super::ai_handlers::ai_list_ai_tasks_handler),
        )
        .route(
            "/_api/database/{db}/ai/tasks/{id}",
            get(super::ai_handlers::ai_get_ai_task_handler),
        )
        .route(
            "/_api/database/{db}/ai/tasks/{id}/claim",
            post(super::ai_handlers::claim_ai_task_handler),
        )
        .route(
            "/_api/database/{db}/ai/tasks/{id}/complete",
            post(super::ai_handlers::complete_ai_task_handler),
        )
        .route(
            "/_api/database/{db}/ai/tasks/{id}/fail",
            post(super::ai_handlers::fail_ai_task_handler),
        )
        // Generic AI content generation
        .route(
            "/_api/database/{db}/ai/generate",
            post(super::ai_handlers::generate_content_handler),
        )
        // AI Agent routes
        .route(
            "/_api/database/{db}/ai/agents",
            get(super::ai_handlers::ai_list_agents_handler),
        )
        .route(
            "/_api/database/{db}/ai/agents",
            post(super::ai_handlers::ai_register_agent_handler),
        )
        .route(
            "/_api/database/{db}/ai/agents/{id}",
            get(super::ai_handlers::ai_get_agent_handler),
        )
        .route(
            "/_api/database/{db}/ai/agents/{id}",
            put(super::ai_handlers::ai_update_agent_handler),
        )
        .route(
            "/_api/database/{db}/ai/agents/{id}",
            delete(super::ai_handlers::ai_unregister_agent_handler),
        )
        .route(
            "/_api/database/{db}/ai/agents/{id}/heartbeat",
            post(super::ai_handlers::ai_agent_heartbeat_handler),
        )
        // AI Marketplace routes
        .route(
            "/_api/database/{db}/ai/marketplace/discover",
            get(super::ai_handlers::ai_discover_agents_handler),
        )
        .route(
            "/_api/database/{db}/ai/marketplace/agent/{id}/reputation",
            get(super::ai_handlers::ai_get_agent_reputation_handler),
        )
        .route(
            "/_api/database/{db}/ai/marketplace/select",
            post(super::ai_handlers::ai_select_agent_handler),
        )
        .route(
            "/_api/database/{db}/ai/marketplace/rankings",
            get(super::ai_handlers::ai_get_agent_rankings_handler),
        )
        // AI Learning routes
        .route(
            "/_api/database/{db}/ai/learning/feedback",
            get(super::ai_handlers::ai_list_feedback_handler),
        )
        .route(
            "/_api/database/{db}/ai/learning/feedback/{id}",
            get(super::ai_handlers::ai_get_feedback_handler),
        )
        .route(
            "/_api/database/{db}/ai/learning/patterns",
            get(super::ai_handlers::ai_list_patterns_handler),
        )
        .route(
            "/_api/database/{db}/ai/learning/patterns/{id}",
            get(super::ai_handlers::ai_get_pattern_handler),
        )
        .route(
            "/_api/database/{db}/ai/learning/process",
            post(super::ai_handlers::ai_process_feedback_handler),
        )
        .route(
            "/_api/database/{db}/ai/learning/recommendations",
            get(super::ai_handlers::ai_get_recommendations_handler),
        )
        // AI Recovery routes
        .route(
            "/_api/database/{db}/ai/recovery/status",
            get(super::ai_handlers::ai_get_recovery_status_handler),
        )
        .route(
            "/_api/database/{db}/ai/recovery/task/{id}/retry",
            post(super::ai_handlers::ai_retry_task_handler),
        )
        .route(
            "/_api/database/{db}/ai/recovery/agent/{id}/reset",
            post(super::ai_handlers::ai_reset_circuit_breaker_handler),
        )
        .route(
            "/_api/database/{db}/ai/recovery/events",
            get(super::ai_handlers::ai_list_recovery_events_handler),
        )
        // AI Validation routes
        .route(
            "/_api/ai/validate",
            post(super::ai_handlers::run_validation_handler),
        )
        .route(
            "/_api/ai/validate/quick",
            get(super::ai_handlers::run_quick_validation_handler),
        )
        // Live Query Token (short-lived token for WebSocket connections)
        .route("/_api/livequery/token", get(livequery_token_handler))
        // SQL compatibility layer
        .route(
            "/_api/database/{db}/sql",
            post(super::sql_handlers::execute_sql_handler),
        )
        .route(
            "/_api/sql/translate",
            post(super::sql_handlers::translate_sql_handler),
        )
        // Offline sync routes
        .route("/_api/sync/session", post(register_sync_session))
        .route("/_api/sync/pull", post(pull_changes))
        .route("/_api/sync/push", post(push_changes))
        .route("/_api/sync/ack", post(acknowledge_changes))
        .route("/_api/sync/conflicts", get(list_conflicts))
        .route("/_api/sync/resolve", post(resolve_conflict))
        // Per-database authorization. Added BEFORE the auth layer so that
        // auth (added last = outermost) runs first and provides Claims.
        // Routes without a {db} path param pass through and rely on their
        // handlers' own permission checks.
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::server::authz_middleware::db_authz_middleware,
        ))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::server::auth::auth_middleware,
        ));

    // Combine with public routes
    Router::new()
        .route("/auth/login", post(login_handler))
        // Health check endpoint for cluster node monitoring (no auth required)
        .route("/_api/health", get(health_check_handler))
        // Prometheus metrics endpoint (no auth required)
        .route("/metrics", get(super::metrics::metrics_handler))
        // Internal cluster endpoints (use cluster secret, no user auth)
        .route("/_api/cluster/cleanup", post(cluster_cleanup))
        .route("/_api/cluster/reshard", post(cluster_reshard))
        // Internal Blob Replication endpoint
        .route(
            "/_internal/blob/replicate/{db}/{collection}/{key}",
            post(crate::sync::blob_replication::receive_blob_replication)
                .layer(DefaultBodyLimit::max(500 * 1024 * 1024)),
        )
        // Internal Blob Chunk fetch endpoint
        .route(
            "/_internal/blob/replicate/{db}/{collection}/{key}/chunk/{chunk_idx}",
            get(crate::sync::blob_replication::get_blob_chunk),
        )
        // Internal Blob Upload endpoint (for shard coordinator forwarding)
        .route(
            "/_internal/blob/upload/{db}/{collection}",
            post(crate::sync::blob_replication::receive_blob_upload)
                .layer(DefaultBodyLimit::max(500 * 1024 * 1024)),
        )
        // WebSocket route (outside auth middleware - uses token in query param)
        .route("/_api/cluster/status/ws", get(cluster_status_ws))
        .route("/_api/ws/changefeed", get(ws_changefeed_handler))
        // Service-based Lua API endpoints: /api/{db}/{service}/{path}
        // (public, service/script handles own auth if needed)
        .route(
            "/api/{db}/{service}",
            get(super::script_handlers::execute_service_script_handler),
        )
        .route(
            "/api/{db}/{service}",
            post(super::script_handlers::execute_service_script_handler),
        )
        .route(
            "/api/{db}/{service}",
            put(super::script_handlers::execute_service_script_handler),
        )
        .route(
            "/api/{db}/{service}",
            delete(super::script_handlers::execute_service_script_handler),
        )
        .route(
            "/api/{db}/{service}/{*path}",
            get(super::script_handlers::execute_service_script_handler),
        )
        .route(
            "/api/{db}/{service}/{*path}",
            post(super::script_handlers::execute_service_script_handler),
        )
        .route(
            "/api/{db}/{service}/{*path}",
            put(super::script_handlers::execute_service_script_handler),
        )
        .route(
            "/api/{db}/{service}/{*path}",
            delete(super::script_handlers::execute_service_script_handler),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::server::auth::permissive_auth_middleware,
        ))
        .merge(api_routes)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            trace_context_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            request_counter_middleware,
        ))
        .with_state(state)
        // Global request body limit: 10MB default (import/blob have 500MB override)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .layer(TraceLayer::new_for_http())
        .layer(
            CompressionLayer::new()
                .gzip(true)
                .zstd(true)
                .no_br()
                .no_deflate()
                // NOTE: `compress_when` REPLACES the default predicate, so we
                // explicitly AND our extra rules onto `DefaultPredicate`
                // (which skips responses smaller than 32 bytes, gRPC, images,
                // and SSE). The closure adds: don't bother gzip-ing 4xx/5xx
                // replies (avoids the encode + Content-Length negotiation on
                // every error), and skip already-compressed video/audio.
                .compress_when({
                    use tower_http::compression::predicate::{
                        DefaultPredicate, Predicate, SizeAbove,
                    };
                    // Gzip has a fixed per-response CPU cost that dwarfs its
                    // benefit on small bodies: measured on the cursor endpoint,
                    // compressing a ~300B JSON response cost ~216µs of server
                    // CPU per request — 65% of the entire request envelope —
                    // and a 65B health response actually *grew* to 88B. Any
                    // client that sends Accept-Encoding (every browser, most
                    // load generators) silently paid it on every call: the
                    // 1-doc query path measured 42.7k req/s with compression
                    // and 103k req/s without. DefaultPredicate's floor is only
                    // 32 bytes, so raise it: don't compress anything below
                    // SOLIDB_GZIP_MIN_BYTES (default 4096 — below that, even a
                    // 3:1 ratio saves at most ~2.7KB per response, well under
                    // one MTU, while the CPU cost stays the same).
                    let min_bytes: u16 = std::env::var("SOLIDB_GZIP_MIN_BYTES")
                        .ok()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(4096);
                    DefaultPredicate::new().and(SizeAbove::new(min_bytes)).and(
                        |status: axum::http::StatusCode,
                         _version: axum::http::Version,
                         headers: &axum::http::HeaderMap,
                         _extensions: &axum::http::Extensions| {
                            if status.is_client_error() || status.is_server_error() {
                                return false;
                            }
                            if let Some(ct) = headers.get(axum::http::header::CONTENT_TYPE) {
                                if let Ok(s) = ct.to_str() {
                                    // Already-compressed formats not covered
                                    // by DefaultPredicate.
                                    if s.starts_with("video/") || s.starts_with("audio/") {
                                        return false;
                                    }
                                }
                            }
                            true
                        },
                    )
                }),
        )
        .layer({
            use axum::http::header;
            use tower_http::cors::AllowOrigin;

            let allowed_origins = get_cors_allowed_origins();
            let is_wildcard = allowed_origins == ["*"];

            // Browsers reject `Access-Control-Allow-Origin: *` combined with
            // `Access-Control-Allow-Credentials: true`. Drop credentials in
            // wildcard mode rather than emit a config that browsers ignore.
            // tower-http likewise rejects `Access-Control-Allow-Headers: *`
            // alongside credentials, so use an explicit allowlist when we
            // need to emit credentials.
            let mut cors = CorsLayer::new()
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PUT,
                    Method::DELETE,
                    Method::OPTIONS,
                ])
                .expose_headers([header::ACCEPT, header::CONTENT_TYPE])
                .max_age(Duration::from_secs(86400));
            if is_wildcard {
                cors = cors.allow_headers(AllowHeaders::any());
            } else {
                cors = cors
                    .allow_headers([
                        header::AUTHORIZATION,
                        header::CONTENT_TYPE,
                        header::ACCEPT,
                        header::HeaderName::from_static("x-api-key"),
                        header::HeaderName::from_static("x-cluster-secret"),
                        header::HeaderName::from_static("x-shard-direct"),
                        header::HeaderName::from_static("x-scatter-gather"),
                    ])
                    .allow_credentials(true);
            }

            if allowed_origins.is_empty() {
                // No explicit origins — deny any cross-origin request.
                cors = cors.allow_origin(AllowOrigin::predicate(|_, _| false));
            } else if is_wildcard {
                tracing::warn!("CORS wildcard mode - allowing any origin");
                cors = cors.allow_origin(AllowOrigin::any());
            } else {
                let origins: Vec<axum::http::header::HeaderValue> = allowed_origins
                    .iter()
                    .filter_map(|o| {
                        o.parse().ok().or_else(|| {
                            tracing::warn!("Failed to parse CORS origin '{}' - skipping", o);
                            None
                        })
                    })
                    .collect();
                if origins.is_empty() {
                    // Every configured origin was unparseable: deny rather than
                    // silently fall back to wildcard.
                    tracing::warn!(
                        "All configured CORS origins failed to parse - denying cross-origin"
                    );
                    cors = cors.allow_origin(AllowOrigin::predicate(|_, _| false));
                } else {
                    cors = cors.allow_origin(AllowOrigin::list(origins));
                }
            }
            cors
        })
}
