//! Scripting engine types

use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;

pub use super::auth::ScriptUser;

/// Service metadata stored in _services collection
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Service {
    /// Service identifier (e.g., "users", "auth")
    #[serde(rename = "_key")]
    pub key: String,
    /// Human-readable name
    pub name: String,
    /// Optional description
    pub description: Option<String>,
    /// API version (e.g., "1.0.0")
    pub version: Option<String>,
    /// Database this service belongs to
    pub database: String,
    /// Whether this service is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Default auth requirement for scripts in this service
    #[serde(default)]
    pub require_auth: bool,
    /// Creation timestamp
    pub created_at: String,
    /// Last modified timestamp
    pub updated_at: String,
}

fn default_enabled() -> bool {
    true
}

/// Context passed to Lua scripts containing request information
#[derive(Debug, Clone)]
pub struct ScriptContext {
    /// HTTP method (GET, POST, PUT, DELETE)
    pub method: String,
    /// Request path (after /api/custom/)
    pub path: String,
    /// Query parameters
    pub query_params: HashMap<String, String>,
    /// URL parameters (e.g., :id)
    pub params: HashMap<String, String>,
    /// Request headers
    pub headers: HashMap<String, String>,
    /// Request body (parsed as JSON if applicable)
    pub body: Option<JsonValue>,
    /// Whether this is a WebSocket connection
    pub is_websocket: bool,
    /// Current authenticated user (if any)
    pub user: ScriptUser,
}

/// Script metadata stored in _system/_scripts
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Script {
    #[serde(rename = "_key")]
    pub key: String,
    /// Human-readable name
    pub name: String,
    /// HTTP methods this script handles (e.g., ["GET", "POST"])
    pub methods: Vec<String>,
    /// URL path pattern (e.g., "users/:id" or "hello")
    pub path: String,
    /// Database this script belongs to
    #[serde(default = "default_database")]
    pub database: String,
    /// Service this script belongs to (required)
    #[serde(default = "default_service")]
    pub service: String,
    /// Collection this script is scoped to (optional)
    pub collection: Option<String>,
    /// The Lua source code
    pub code: String,
    /// Optional description
    pub description: Option<String>,
    /// Creation timestamp
    pub created_at: String,
    /// Last modified timestamp
    pub updated_at: String,
}

fn default_database() -> String {
    "_system".to_string()
}

fn default_service() -> String {
    "default".to_string()
}

/// Runtime statistics for the script engine
#[derive(Debug, Default)]
pub struct ScriptStats {
    /// Number of HTTP scripts currently executing
    pub active_scripts: AtomicUsize,
    /// Number of active WebSocket connections
    pub active_ws: AtomicUsize,
    /// Total number of HTTP scripts executed since start
    pub total_scripts_executed: AtomicUsize,
    /// Total number of WebSocket connections handled since start
    pub total_ws_connections: AtomicUsize,
}

/// Result from script execution
#[derive(Debug)]
pub struct ScriptResult {
    pub status: u16,
    pub body: JsonValue,
    pub headers: HashMap<String, String>,
}
