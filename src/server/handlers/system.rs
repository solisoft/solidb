use crate::server::cursor_store::CursorStore;
use crate::scripting::ScriptStats;
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
}

impl AppState {
    /// Get the cluster secret from the keyfile for inter-node HTTP authentication
    pub fn cluster_secret(&self) -> String {
        self.storage
            .cluster_config()
            .and_then(|c| c.keyfile.clone())
            .unwrap_or_default()
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
