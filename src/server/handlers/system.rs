use crate::error::DbError;
use crate::scripting::engine::{LuaPool, ScriptCache, ScriptIndex};
use crate::scripting::ScriptStats;
use crate::server::cursor_store::CursorStore;
use crate::server::service_cache::ServiceCache;
use crate::server::upload_session::UploadSessionStore;
use crate::storage::StorageEngine;
use axum::response::Json;
use serde_json::Value;
use std::sync::Arc;

/// Protected system collections that cannot be deleted or modified via standard API
pub const PROTECTED_COLLECTIONS: [&str; 2] = ["_admins", "_api_keys"];

/// Check if a collection is a protected system collection
#[inline]
pub fn is_protected_collection(db_name: &str, coll_name: &str) -> bool {
    db_name == "_system" && PROTECTED_COLLECTIONS.contains(&coll_name)
}

/// Check if a collection is a physical shard (ends with _sN where N is a number)
/// Physical shards are implementation details and should be hidden from users
#[inline]
pub fn is_physical_shard_collection(name: &str) -> bool {
    if let Some(pos) = name.rfind("_s") {
        let suffix = &name[pos + 2..];
        !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
    } else {
        false
    }
}

/// Sanitize a filename for use in Content-Disposition header to prevent header injection
/// Removes/replaces: quotes, backslashes, newlines, carriage returns, and non-ASCII characters
pub fn sanitize_filename(filename: &str) -> String {
    filename
        .chars()
        .filter(|c| c.is_ascii() && *c != '"' && *c != '\\' && *c != '\n' && *c != '\r')
        .collect::<String>()
        .trim()
        .to_string()
}

/// Calculate the size of a directory recursively
pub fn get_dir_size(path: impl AsRef<std::path::Path>) -> std::io::Result<u64> {
    let mut size = 0;
    if path.as_ref().is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                size += get_dir_size(entry.path())?;
            } else {
                size += metadata.len();
            }
        }
    } else {
        size = std::fs::metadata(path)?.len();
    }
    Ok(size)
}

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<StorageEngine>,
    pub cursor_store: CursorStore,
    // New Architecture Components
    pub cluster_manager: Option<Arc<crate::cluster::manager::ClusterManager>>,
    pub replication_log: Option<Arc<crate::sync::log::SyncLog>>,
    pub shard_coordinator: Option<Arc<crate::sharding::ShardCoordinator>>,
    pub startup_time: std::time::Instant,
    pub request_counter: Arc<std::sync::atomic::AtomicU64>,
    pub query_counter: Arc<std::sync::atomic::AtomicU64>,
    pub write_counter: Arc<std::sync::atomic::AtomicU64>,
    pub system_monitor: Arc<std::sync::Mutex<sysinfo::System>>,
    pub queue_worker: Option<Arc<crate::queue::QueueWorker>>,
    pub script_stats: Arc<ScriptStats>,
    // Stream Processing Manager
    pub stream_manager: Option<Arc<crate::stream::StreamManager>>,
    // RBAC permission cache
    pub permission_cache: crate::server::permission_cache::PermissionCache,
    // REPL session store
    pub repl_sessions: crate::server::repl_session::ReplSessionStore,
    // WebSocket Channel Manager for pub/sub and presence
    pub channel_manager: Arc<crate::scripting::ChannelManager>,
    // Sync session manager for offline-first client sync
    pub sync_session_manager: Option<Arc<crate::sync::SyncSessionManager>>,
    // Lua VM pool. `None` when `--no-lua` / `SOLIDB_NO_LUA` is set.
    pub lua_pool: Option<Arc<LuaPool>>,
    // Script bytecode cache
    pub script_cache: Arc<ScriptCache>,
    // Script index for fast route lookup
    pub script_index: Arc<ScriptIndex>,
    // Service metadata cache to avoid RocksDB reads on every script request
    pub service_cache: Arc<ServiceCache>,
    // Blob rebalance worker for cluster maintenance
    pub blob_rebalance_worker: Option<Arc<crate::sharding::BlobRebalanceWorker>>,
    // Resumable blob upload session store
    pub upload_session_store: UploadSessionStore,
}

impl AppState {
    /// Get the cluster secret from the keyfile for inter-node HTTP authentication
    pub fn cluster_secret(&self) -> String {
        self.storage
            .cluster_config()
            .and_then(|c| c.keyfile.clone())
            .unwrap_or_default()
    }

    /// Require that a request carries the cluster secret, and return it.
    ///
    /// For endpoints that are only ever called node-to-node but live on the
    /// public API router. `auth_middleware`'s `X-Shard-Direct` bypass grants
    /// admin to whoever presents this secret, so any handler that *sends* the
    /// secret onwards must first prove its caller already had it — otherwise
    /// the handler is a secret-exfiltration oracle for anyone who can reach
    /// the route.
    ///
    /// Fails closed: a node with no keyfile configured refuses the request
    /// rather than accepting an empty secret, matching `auth_middleware`.
    pub fn require_cluster_secret(
        &self,
        headers: &axum::http::HeaderMap,
    ) -> Result<String, DbError> {
        let expected = self.cluster_secret();
        if expected.is_empty() {
            return Err(DbError::Forbidden(
                "Internal cluster endpoint: no keyfile configured on this node".to_string(),
            ));
        }
        let provided = headers
            .get("X-Cluster-Secret")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        if !crate::server::auth::constant_time_eq(expected.as_bytes(), provided.as_bytes()) {
            return Err(DbError::Forbidden(
                "Internal cluster endpoint requires a valid cluster secret".to_string(),
            ));
        }
        Ok(expected)
    }

    /// Resolve a caller-supplied peer address against known cluster members.
    ///
    /// Inter-node handlers that dial an address from a request body must not
    /// trust that address: it decides where this node's credentials are sent.
    /// Only addresses this node already knows as members (or its own) are
    /// accepted.
    pub fn require_known_peer(&self, address: &str) -> Result<(), DbError> {
        let manager = self.cluster_manager.as_ref().ok_or_else(|| {
            DbError::Forbidden("Cluster operations are not enabled on this node".to_string())
        })?;

        let addr = address.trim();
        if addr.is_empty() {
            return Err(DbError::BadRequest("Empty peer address".to_string()));
        }

        let known = manager.get_local_address() == addr
            || manager
                .state()
                .get_all_members()
                .iter()
                .any(|m| m.node.api_address == addr || m.node.address == addr);

        if !known {
            tracing::warn!(
                "CLUSTER: refusing peer address '{}' — not a known cluster member",
                addr
            );
            return Err(DbError::Forbidden(format!(
                "Address '{}' is not a known cluster member",
                addr
            )));
        }
        Ok(())
    }
}

// ==================== Health Check Handler ====================

/// Simple health check endpoint for cluster node monitoring
/// Returns 200 OK if the node is alive and accepting requests
pub async fn health_check_handler() -> Json<Value> {
    Json(serde_json::json!({
        "status": "ok",
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}
