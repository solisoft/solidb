//! Lua Scripting Engine for Custom API Endpoints
//!
//! This module provides embedded Lua scripting capabilities that allow users
//! to create custom API endpoints with full access to database operations.

use mlua::{FromLua, Lua, Result as LuaResult, Value as LuaValue};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::error::DbError;
use crate::sdbql::{parse, QueryExecutor};
use crate::storage::StorageEngine;
use crate::stream::StreamManager;

/// Maximum allowed regex pattern length to prevent DoS attacks
const MAX_REGEX_PATTERN_LEN: usize = 1024;

/// Maximum regex compiled size (1MB) to prevent memory exhaustion
const MAX_REGEX_SIZE: usize = 1 << 20;

/// Create a regex with safety limits to prevent ReDoS attacks.
fn safe_regex(pattern: &str) -> Result<regex::Regex, String> {
    if pattern.len() > MAX_REGEX_PATTERN_LEN {
        return Err(format!(
            "Regex pattern too long: {} bytes (max {})",
            pattern.len(),
            MAX_REGEX_PATTERN_LEN
        ));
    }

    regex::RegexBuilder::new(pattern)
        .size_limit(MAX_REGEX_SIZE)
        .build()
        .map_err(|e| e.to_string())
}
use futures::{SinkExt, StreamExt};

// Import modules
mod ai_bindings;
mod auth;
pub mod channel_manager;
mod dev_tools;
mod error_handling;
mod file_handling;
mod http_helpers;
mod string_utils;
mod validation;
pub use auth::ScriptUser;
pub use channel_manager::ChannelManager;
use dev_tools::*;
use error_handling::*;
use file_handling::*;
use http_helpers::*;
use string_utils::*;
use validation::*;

// Crypto imports
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use base64::Engine;
use hmac::Mac;
use rand::RngCore;
use sha2::Digest;

// Custom JWT implementation for scripting
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
struct Header {
    alg: String,
    typ: String,
}

impl Header {
    fn default() -> Self {
        Self {
            alg: "HS256".to_string(),
            typ: "JWT".to_string(),
        }
    }
}

#[derive(Debug)]
struct Validation;

impl Validation {
    fn default() -> Self {
        Self
    }
}

#[derive(Debug)]
struct EncodingKey(Vec<u8>);

impl EncodingKey {
    fn from_secret(secret: &[u8]) -> Self {
        Self(secret.to_vec())
    }
}

#[derive(Debug)]
struct DecodingKey(Vec<u8>);

impl DecodingKey {
    fn from_secret(secret: &[u8]) -> Self {
        Self(secret.to_vec())
    }
}

fn encode<T: serde::Serialize>(
    _header: &Header,
    claims: &T,
    key: &EncodingKey,
) -> Result<String, String> {
    // JWT Header: {"alg":"HS256","typ":"JWT"}
    let header = r#"{"alg":"HS256","typ":"JWT"}"#;

    // Base64url encode header
    let header_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(header);

    // Serialize and encode claims
    let claims_json =
        serde_json::to_string(claims).map_err(|e| format!("JWT encode failed: {}", e))?;
    let claims_b64 =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims_json.as_bytes());

    // Create signing input
    let signing_input = format!("{}.{}", header_b64, claims_b64);

    // Sign with HMAC-SHA256
    let signature = sign_hmac_sha256(&signing_input, &key.0)?;

    // Combine into JWT format: header.claims.signature
    Ok(format!("{}.{}.{}", header_b64, claims_b64, signature))
}

fn decode<T: serde::de::DeserializeOwned>(
    token: &str,
    key: &DecodingKey,
    _validation: &Validation,
) -> Result<TokenData<T>, String> {
    // Split JWT into parts
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("Invalid JWT format".to_string());
    }

    let (header_b64, claims_b64, signature_b64) = (parts[0], parts[1], parts[2]);

    // Verify header (should be {"alg":"HS256","typ":"JWT"})
    let header_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(header_b64)
        .map_err(|_| "Invalid JWT header".to_string())?;
    let header_str =
        String::from_utf8(header_bytes).map_err(|_| "Invalid JWT header encoding".to_string())?;

    if !header_str.contains(r#""alg":"HS256""#) || !header_str.contains(r#""typ":"JWT""#) {
        return Err("Unsupported JWT algorithm or type".to_string());
    }

    // Verify signature
    let signing_input = format!("{}.{}", header_b64, claims_b64);
    let expected_signature = sign_hmac_sha256(&signing_input, &key.0)?;

    if expected_signature != signature_b64 {
        return Err("Invalid JWT signature".to_string());
    }

    // Decode claims
    let claims_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(claims_b64)
        .map_err(|_| "Invalid JWT claims".to_string())?;

    let claims: T = serde_json::from_slice(&claims_bytes)
        .map_err(|_| "Invalid JWT claims format".to_string())?;

    Ok(TokenData {
        header: Header::default(),
        claims,
    })
}

#[derive(Debug)]
struct TokenData<T> {
    #[allow(dead_code)]
    header: Header,
    claims: T,
}

fn sign_hmac_sha256(data: &str, secret: &[u8]) -> Result<String, String> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret).map_err(|e| format!("HMAC init failed: {}", e))?;
    mac.update(data.as_bytes());

    let result = mac.finalize();
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(result.into_bytes()))
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

/// Lua scripting engine
pub struct ScriptEngine {
    storage: Arc<StorageEngine>,
    queue_notifier: Option<broadcast::Sender<()>>,
    stream_manager: Option<Arc<StreamManager>>,
    channel_manager: Option<Arc<ChannelManager>>,
    stats: Arc<ScriptStats>,
}

impl ScriptEngine {
    /// Create a new script engine with access to the storage layer
    pub fn new(storage: Arc<StorageEngine>, stats: Arc<ScriptStats>) -> Self {
        Self {
            storage,
            queue_notifier: None,
            stream_manager: None,
            channel_manager: None,
            stats,
        }
    }

    pub fn with_queue_notifier(mut self, notifier: broadcast::Sender<()>) -> Self {
        self.queue_notifier = Some(notifier);
        self
    }
    
    pub fn with_stream_manager(mut self, manager: Arc<StreamManager>) -> Self {
        self.stream_manager = Some(manager);
        self
    }

    pub fn with_channel_manager(mut self, manager: Arc<ChannelManager>) -> Self {
        self.channel_manager = Some(manager);
        self
    }

    /// Execute a Lua script with the given context
    pub async fn execute(
        &self,
        script: &Script,
        db_name: &str,
        context: &ScriptContext,
    ) -> Result<ScriptResult, DbError> {
        self.stats.active_scripts.fetch_add(1, Ordering::SeqCst);
        self.stats
            .total_scripts_executed
            .fetch_add(1, Ordering::SeqCst);

        // Ensure active counter is decremented even on panic or early return
        struct ActiveScriptGuard(Arc<ScriptStats>);
        impl Drop for ActiveScriptGuard {
            fn drop(&mut self) {
                self.0.active_scripts.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let _guard = ActiveScriptGuard(self.stats.clone());

        let lua = Lua::new();

        // Secure environment: Remove unsafe standard libraries and functions
        let globals = lua.globals();
        globals
            .set("os", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure os: {}", e)))?;
        globals
            .set("io", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure io: {}", e)))?;
        globals
            .set("debug", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure debug: {}", e)))?;
        globals
            .set("package", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure package: {}", e)))?;
        globals
            .set("dofile", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure dofile: {}", e)))?;
        globals
            .set("load", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure load: {}", e)))?;
        globals
            .set("loadfile", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure loadfile: {}", e)))?;
        globals
            .set("require", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure require: {}", e)))?;

        // Set up the Lua environment
        self.setup_lua_globals(&lua, db_name, context, Some((&script.key, &script.name)))?;

        // Execute the script
        let chunk = lua.load(&script.code);

        match chunk.eval_async::<LuaValue>().await {
            Ok(result) => {
                // Convert Lua result to JSON
                let json_result = self.lua_to_json(&lua, result)?;
                Ok(ScriptResult {
                    status: 200,
                    body: json_result,
                    headers: HashMap::new(),
                })
            }
            Err(e) => Err(DbError::InternalError(format!("Lua error: {}", e))),
        }
    }

    /// Execute a Lua script as a WebSocket handler
    pub async fn execute_ws(
        &self,
        script: &Script,
        db_name: &str,
        context: &ScriptContext,
        ws: axum::extract::ws::WebSocket,
    ) -> Result<(), DbError> {
        use channel_manager::{ChannelEvent, ChannelManager, ConnectionId};

        self.stats.active_ws.fetch_add(1, Ordering::SeqCst);
        self.stats
            .total_ws_connections
            .fetch_add(1, Ordering::SeqCst);

        // Ensure active counter is decremented even on panic or early return
        struct ActiveWsGuard(Arc<ScriptStats>);
        impl Drop for ActiveWsGuard {
            fn drop(&mut self) {
                self.0.active_ws.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let _guard = ActiveWsGuard(self.stats.clone());

        // Register connection with channel manager for pub/sub and presence
        let channel_manager = self.channel_manager.clone();
        let (conn_id, event_rx): (ConnectionId, tokio::sync::mpsc::Receiver<ChannelEvent>) =
            if let Some(cm) = &channel_manager {
                cm.register_connection(db_name)
            } else {
                // Create a dummy receiver if no channel manager
                let (_tx, rx) = tokio::sync::mpsc::channel(1);
                (uuid::Uuid::new_v4().to_string(), rx)
            };

        // Guard for automatic connection cleanup
        struct ConnectionGuard {
            conn_id: ConnectionId,
            channel_manager: Option<Arc<ChannelManager>>,
        }
        impl Drop for ConnectionGuard {
            fn drop(&mut self) {
                if let Some(cm) = &self.channel_manager {
                    cm.unregister_connection(&self.conn_id);
                }
            }
        }
        let _conn_guard = ConnectionGuard {
            conn_id: conn_id.clone(),
            channel_manager: channel_manager.clone(),
        };

        let lua = Lua::new();

        // Secure environment
        let globals = lua.globals();
        globals
            .set("os", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure os: {}", e)))?;
        globals
            .set("io", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure io: {}", e)))?;
        globals
            .set("debug", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure debug: {}", e)))?;
        globals
            .set("package", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure package: {}", e)))?;
        globals
            .set("dofile", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure dofile: {}", e)))?;
        globals
            .set("load", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure load: {}", e)))?;
        globals
            .set("loadfile", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure loadfile: {}", e)))?;
        globals
            .set("require", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure require: {}", e)))?;

        // Set up the Lua environment
        self.setup_lua_globals(&lua, db_name, context, Some((&script.key, &script.name)))?;

        // Set up WebSocket specific globals
        let ws_table = lua
            .create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create ws table: {}", e)))?;

        // Split WebSocket into sink and stream
        let (mut sink, receiver) = ws.split();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<axum::extract::ws::Message>(100);

        // Task to forward messages from channel to WebSocket sink
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if sink.send(msg).await.is_err() {
                    break;
                }
            }
        });

        // Heartbeat task: Send a Ping every 30 seconds to keep the connection alive
        let tx_heartbeat = tx.clone();
        let heartbeat_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            // First tick happens immediately
            interval.tick().await;
            loop {
                interval.tick().await;
                if tx_heartbeat
                    .send(axum::extract::ws::Message::Ping(vec![].into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        let receiver_arc = Arc::new(tokio::sync::Mutex::new(receiver));
        let event_rx_arc = Arc::new(tokio::sync::Mutex::new(event_rx));

        // ws.send(data)
        let tx_send = tx.clone();
        let send_fn = lua
            .create_async_function(move |_, data: String| {
                let tx = tx_send.clone();
                async move {
                    tx.send(axum::extract::ws::Message::Text(data.into()))
                        .await
                        .map_err(|e| mlua::Error::RuntimeError(format!("WS send error: {}", e)))?;
                    Ok(())
                }
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create ws.send: {}", e)))?;
        ws_table
            .set("send", send_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set ws.send: {}", e)))?;

        // ws.recv() -> string or nil
        let ws_recv_clone = receiver_arc.clone();
        let recv_fn = lua
            .create_async_function(move |lua, (): ()| {
                let stream_inner = ws_recv_clone.clone();
                async move {
                    let mut stream = stream_inner.lock().await;
                    loop {
                        match stream.next().await {
                            Some(Ok(axum::extract::ws::Message::Text(t))) => {
                                return Ok(LuaValue::String(lua.create_string(t.as_bytes())?))
                            }
                            Some(Ok(axum::extract::ws::Message::Binary(b))) => {
                                return Ok(LuaValue::String(lua.create_string(b.as_ref())?))
                            }
                            Some(Ok(axum::extract::ws::Message::Close(_)))
                            | None
                            | Some(Err(_)) => return Ok(LuaValue::Nil),
                            Some(Ok(axum::extract::ws::Message::Pong(_)))
                            | Some(Ok(axum::extract::ws::Message::Ping(_))) => continue, // Ignore heartbeats
                        }
                    }
                }
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create ws.recv: {}", e)))?;
        ws_table
            .set("recv", recv_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set ws.recv: {}", e)))?;

        // ws.recv_any(timeout_ms) -> (msg, type) or nil
        // Returns messages from both WebSocket and channel events
        let ws_recv_any_clone = receiver_arc.clone();
        let event_rx_clone = event_rx_arc.clone();
        let recv_any_fn = lua
            .create_async_function(move |lua, timeout_ms: Option<u64>| {
                let ws_stream = ws_recv_any_clone.clone();
                let event_rx = event_rx_clone.clone();
                async move {
                    let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(30000));

                    tokio::select! {
                        biased;

                        // Check channel events first (they're usually more important for real-time)
                        result = async {
                            event_rx.lock().await.recv().await
                        } => {
                            match result {
                                Some(ChannelEvent::Message(msg)) => {
                                    let msg_table = lua.create_table()?;
                                    msg_table.set("channel", msg.channel.as_str())?;
                                    msg_table.set("data", json_to_lua(&lua, &msg.data)?)?;
                                    msg_table.set("timestamp", msg.timestamp)?;
                                    if let Some(sender) = &msg.sender_id {
                                        msg_table.set("sender_id", sender.as_str())?;
                                    }
                                    let result_table = lua.create_table()?;
                                    result_table.set(1, msg_table)?;
                                    result_table.set(2, "channel")?;
                                    Ok(LuaValue::Table(result_table))
                                }
                                Some(ChannelEvent::Presence(event)) => {
                                    let event_table = lua.create_table()?;
                                    event_table.set("event_type", event.event_type.to_string())?;
                                    event_table.set("channel", event.channel.as_str())?;
                                    event_table.set("user_info", json_to_lua(&lua, &event.user_info)?)?;
                                    event_table.set("connection_id", event.connection_id.as_str())?;
                                    event_table.set("timestamp", event.timestamp)?;
                                    let result_table = lua.create_table()?;
                                    result_table.set(1, event_table)?;
                                    result_table.set(2, "presence")?;
                                    Ok(LuaValue::Table(result_table))
                                }
                                None => Ok(LuaValue::Nil),
                            }
                        }

                        // WebSocket message
                        result = async {
                            let mut stream = ws_stream.lock().await;
                            stream.next().await
                        } => {
                            match result {
                                Some(Ok(axum::extract::ws::Message::Text(t))) => {
                                    let result_table = lua.create_table()?;
                                    result_table.set(1, lua.create_string(t.as_bytes())?)?;
                                    result_table.set(2, "ws")?;
                                    Ok(LuaValue::Table(result_table))
                                }
                                Some(Ok(axum::extract::ws::Message::Binary(b))) => {
                                    let result_table = lua.create_table()?;
                                    result_table.set(1, lua.create_string(b.as_ref())?)?;
                                    result_table.set(2, "ws")?;
                                    Ok(LuaValue::Table(result_table))
                                }
                                Some(Ok(axum::extract::ws::Message::Close(_)))
                                | None
                                | Some(Err(_)) => Ok(LuaValue::Nil),
                                Some(Ok(axum::extract::ws::Message::Pong(_)))
                                | Some(Ok(axum::extract::ws::Message::Ping(_))) => {
                                    // Ignore heartbeats, return nil to indicate no user message
                                    Ok(LuaValue::Nil)
                                }
                            }
                        }

                        // Timeout
                        _ = tokio::time::sleep(timeout) => {
                            Ok(LuaValue::Nil)
                        }
                    }
                }
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create ws.recv_any: {}", e)))?;
        ws_table
            .set("recv_any", recv_any_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set ws.recv_any: {}", e)))?;

        // ws.close()
        let tx_close = tx.clone();
        let close_fn = lua
            .create_async_function(move |_, (): ()| {
                let tx = tx_close.clone();
                async move {
                    let _ = tx.send(axum::extract::ws::Message::Close(None)).await;
                    Ok(())
                }
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create ws.close: {}", e)))?;
        ws_table
            .set("close", close_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set ws.close: {}", e)))?;

        // ==================== Channel Operations ====================
        if let Some(cm) = &channel_manager {
            let channel_table = lua
                .create_table()
                .map_err(|e| DbError::InternalError(format!("Failed to create channel table: {}", e)))?;

            // ws.channel.subscribe(channel_name)
            let cm_subscribe = cm.clone();
            let conn_id_sub = conn_id.clone();
            let subscribe_fn = lua
                .create_function(move |_, channel: String| {
                    cm_subscribe
                        .subscribe(&conn_id_sub, &channel)
                        .map_err(|e| mlua::Error::RuntimeError(format!("Subscribe error: {}", e)))?;
                    Ok(true)
                })
                .map_err(|e| DbError::InternalError(format!("Failed to create channel.subscribe: {}", e)))?;
            channel_table
                .set("subscribe", subscribe_fn)
                .map_err(|e| DbError::InternalError(format!("Failed to set channel.subscribe: {}", e)))?;

            // ws.channel.unsubscribe(channel_name)
            let cm_unsub = cm.clone();
            let conn_id_unsub = conn_id.clone();
            let unsubscribe_fn = lua
                .create_function(move |_, channel: String| {
                    cm_unsub.unsubscribe(&conn_id_unsub, &channel);
                    Ok(true)
                })
                .map_err(|e| DbError::InternalError(format!("Failed to create channel.unsubscribe: {}", e)))?;
            channel_table
                .set("unsubscribe", unsubscribe_fn)
                .map_err(|e| DbError::InternalError(format!("Failed to set channel.unsubscribe: {}", e)))?;

            // ws.channel.broadcast(channel_name, data)
            let cm_broadcast = cm.clone();
            let conn_id_bc = conn_id.clone();
            let broadcast_fn = lua
                .create_function(move |_, (channel, data): (String, mlua::Value)| {
                    let json_data = lua_value_to_json(&data)?;
                    cm_broadcast
                        .broadcast(&channel, json_data, Some(&conn_id_bc))
                        .map_err(|e| mlua::Error::RuntimeError(format!("Broadcast error: {}", e)))?;
                    Ok(true)
                })
                .map_err(|e| DbError::InternalError(format!("Failed to create channel.broadcast: {}", e)))?;
            channel_table
                .set("broadcast", broadcast_fn)
                .map_err(|e| DbError::InternalError(format!("Failed to set channel.broadcast: {}", e)))?;

            // ws.channel.list() -> table of subscribed channels
            let cm_list = cm.clone();
            let conn_id_list = conn_id.clone();
            let list_fn = lua
                .create_function(move |lua, ()| {
                    let channels = cm_list.list_subscriptions(&conn_id_list);
                    let table = lua.create_table()?;
                    for (i, ch) in channels.iter().enumerate() {
                        table.set(i + 1, ch.as_str())?;
                    }
                    Ok(table)
                })
                .map_err(|e| DbError::InternalError(format!("Failed to create channel.list: {}", e)))?;
            channel_table
                .set("list", list_fn)
                .map_err(|e| DbError::InternalError(format!("Failed to set channel.list: {}", e)))?;

            ws_table
                .set("channel", channel_table)
                .map_err(|e| DbError::InternalError(format!("Failed to set ws.channel: {}", e)))?;

            // ==================== Presence Operations ====================
            let presence_table = lua
                .create_table()
                .map_err(|e| DbError::InternalError(format!("Failed to create presence table: {}", e)))?;

            // ws.presence.join(channel, user_info)
            let cm_join = cm.clone();
            let conn_id_join = conn_id.clone();
            let join_fn = lua
                .create_function(move |_, (channel, user_info): (String, mlua::Value)| {
                    let json_info = lua_value_to_json(&user_info)?;
                    cm_join
                        .presence_join(&conn_id_join, &channel, json_info)
                        .map_err(|e| mlua::Error::RuntimeError(format!("Presence join error: {}", e)))?;
                    Ok(true)
                })
                .map_err(|e| DbError::InternalError(format!("Failed to create presence.join: {}", e)))?;
            presence_table
                .set("join", join_fn)
                .map_err(|e| DbError::InternalError(format!("Failed to set presence.join: {}", e)))?;

            // ws.presence.leave(channel)
            let cm_leave = cm.clone();
            let conn_id_leave = conn_id.clone();
            let leave_fn = lua
                .create_function(move |_, channel: String| {
                    cm_leave.presence_leave(&conn_id_leave, &channel);
                    Ok(true)
                })
                .map_err(|e| DbError::InternalError(format!("Failed to create presence.leave: {}", e)))?;
            presence_table
                .set("leave", leave_fn)
                .map_err(|e| DbError::InternalError(format!("Failed to set presence.leave: {}", e)))?;

            // ws.presence.list(channel) -> table of users
            let cm_plist = cm.clone();
            let list_presence_fn = lua
                .create_function(move |lua, channel: String| {
                    let users = cm_plist.presence_list(&channel);
                    let table = lua.create_table()?;
                    for (i, user) in users.iter().enumerate() {
                        let user_table = lua.create_table()?;
                        user_table.set("connection_id", user.connection_id.as_str())?;
                        user_table.set("user_info", json_to_lua(lua, &user.user_info)?)?;
                        user_table.set("joined_at", user.joined_at)?;
                        table.set(i + 1, user_table)?;
                    }
                    Ok(table)
                })
                .map_err(|e| DbError::InternalError(format!("Failed to create presence.list: {}", e)))?;
            presence_table
                .set("list", list_presence_fn)
                .map_err(|e| DbError::InternalError(format!("Failed to set presence.list: {}", e)))?;

            ws_table
                .set("presence", presence_table)
                .map_err(|e| DbError::InternalError(format!("Failed to set ws.presence: {}", e)))?;
        }

        let solidb: mlua::Table = globals
            .get("solidb")
            .map_err(|e| DbError::InternalError(format!("Failed to get solidb table: {}", e)))?;
        solidb
            .set("ws", ws_table)
            .map_err(|e| DbError::InternalError(format!("Failed to set solidb.ws: {}", e)))?;

        // Execute the script
        let chunk = lua.load(&script.code);
        let result = match chunk.eval_async::<LuaValue>().await {
            Ok(_) => Ok(()),
            Err(e) => {
                tracing::error!("WebSocket Lua script error: {}", e);
                // Also try to notify the client of the error if possible
                let _ = tx
                    .send(axum::extract::ws::Message::Text(
                        format!("Lua Error: {}", e).into(),
                    ))
                    .await;
                Err(DbError::InternalError(format!("Lua error: {}", e)))
            }
        };

        // Cleanup
        heartbeat_task.abort();
        result
    }

    /// Execute Lua code in REPL mode with variable persistence
    pub async fn execute_repl(
        &self,
        code: &str,
        db_name: &str,
        variables: &HashMap<String, JsonValue>,
        history: &[String],
        output_capture: &mut Vec<String>,
    ) -> Result<(JsonValue, HashMap<String, JsonValue>), DbError> {
        self.stats.active_scripts.fetch_add(1, Ordering::SeqCst);
        self.stats
            .total_scripts_executed
            .fetch_add(1, Ordering::SeqCst);

        // Ensure active counter is decremented even on panic or early return
        struct ActiveScriptGuard(Arc<ScriptStats>);
        impl Drop for ActiveScriptGuard {
            fn drop(&mut self) {
                self.0.active_scripts.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let _guard = ActiveScriptGuard(self.stats.clone());

        let lua = Lua::new();

        // Secure environment: Remove unsafe standard libraries and functions
        let globals = lua.globals();
        globals
            .set("os", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure os: {}", e)))?;
        globals
            .set("io", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure io: {}", e)))?;
        globals
            .set("debug", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure debug: {}", e)))?;
        globals
            .set("package", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure package: {}", e)))?;
        globals
            .set("dofile", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure dofile: {}", e)))?;
        globals
            .set("load", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure load: {}", e)))?;
        globals
            .set("loadfile", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure loadfile: {}", e)))?;
        globals
            .set("require", LuaValue::Nil)
            .map_err(|e| DbError::InternalError(format!("Failed to secure require: {}", e)))?;

        // Create a minimal ScriptContext for REPL (no HTTP context)
        let context = ScriptContext {
            method: "REPL".to_string(),
            path: "repl".to_string(),
            query_params: HashMap::new(),
            params: HashMap::new(),
            headers: HashMap::new(),
            body: None,
            is_websocket: false,
            user: ScriptUser::anonymous(),
        };

        // Set up the Lua environment (script info is None for REPL)
        self.setup_lua_globals(&lua, db_name, &context, None)?;

        // Inject session variables into global scope
        for (name, value) in variables {
            // Check if this is a saved collection handle that needs recreation
            if let JsonValue::Object(ref obj) = value {
                if obj.get("_solidb_handle").and_then(|v| v.as_bool()) == Some(true) {
                    // Recreate collection handle using db:collection()
                    if let Some(coll_name) = obj.get("_name").and_then(|v| v.as_str()) {
                        let recreate_code = format!("{} = db:collection(\"{}\")", name, coll_name);
                        let _ = lua.load(&recreate_code).exec();
                        continue;
                    }
                }
            }
            let lua_val = json_to_lua(&lua, value).map_err(|e| {
                DbError::InternalError(format!("Failed to convert variable '{}': {}", name, e))
            })?;
            globals.set(name.clone(), lua_val).map_err(|e| {
                DbError::InternalError(format!("Failed to inject variable '{}': {}", name, e))
            })?;
        }

        // Replay function definitions from history (functions can't be serialized to JSON)
        // This allows functions defined in previous commands to persist across REPL calls
        for prev_code in history {
            let trimmed = prev_code.trim();
            // Check if this looks like a function definition
            if trimmed.starts_with("function ")
                || trimmed.contains("= function")
                || trimmed.starts_with("local function ")
            {
                // Silently re-execute function definitions
                let _ = lua.load(prev_code).exec();
            }
        }

        // Set up output capture by replacing solidb.log
        let output_clone = Arc::new(std::sync::Mutex::new(output_capture.clone()));
        let output_ref = output_clone.clone();

        let capture_log_fn = lua
            .create_function(move |lua, val: mlua::Value| {
                let msg = match val {
                    mlua::Value::Nil => "nil".to_string(),
                    mlua::Value::Boolean(b) => b.to_string(),
                    mlua::Value::Integer(i) => i.to_string(),
                    mlua::Value::Number(n) => n.to_string(),
                    mlua::Value::String(s) => s
                        .to_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|_| "[invalid string]".to_string()),
                    mlua::Value::Table(t) => {
                        // Simple JSON-like serialization for tables
                        if let Ok(json) = Self::table_to_json_static(lua, t) {
                            serde_json::to_string(&json).unwrap_or_else(|_| "[table]".to_string())
                        } else {
                            "[table]".to_string()
                        }
                    }
                    _ => "[unsupported type]".to_string(),
                };

                // Add to output capture
                if let Ok(mut output) = output_ref.lock() {
                    output.push(msg);
                }

                Ok(())
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create capture log fn: {}", e))
            })?;

        // Update solidb.log with capture version
        let solidb: mlua::Table = globals
            .get("solidb")
            .map_err(|e| DbError::InternalError(format!("Failed to get solidb table: {}", e)))?;
        solidb
            .set("log", capture_log_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set capture log: {}", e)))?;

        // Also add print function that captures output
        let output_print_ref = output_clone.clone();
        let print_fn = lua
            .create_function(move |lua, args: mlua::Variadic<mlua::Value>| {
                let mut parts = Vec::new();
                for val in args {
                    let part = match val {
                        mlua::Value::Nil => "nil".to_string(),
                        mlua::Value::Boolean(b) => b.to_string(),
                        mlua::Value::Integer(i) => i.to_string(),
                        mlua::Value::Number(n) => n.to_string(),
                        mlua::Value::String(s) => s
                            .to_str()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|_| "[invalid string]".to_string()),
                        mlua::Value::Table(t) => {
                            if let Ok(json) = Self::table_to_json_static(lua, t) {
                                serde_json::to_string(&json)
                                    .unwrap_or_else(|_| "[table]".to_string())
                            } else {
                                "[table]".to_string()
                            }
                        }
                        _ => "[unsupported type]".to_string(),
                    };
                    parts.push(part);
                }

                if let Ok(mut output) = output_print_ref.lock() {
                    output.push(parts.join("\t"));
                }

                Ok(())
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create print fn: {}", e)))?;

        globals
            .set("print", print_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set print: {}", e)))?;

        // Execute the code
        let chunk = lua.load(code);

        let result = match chunk.eval_async::<LuaValue>().await {
            Ok(result) => {
                // Convert Lua result to JSON
                let json_result = self.lua_to_json(&lua, result)?;
                Ok(json_result)
            }
            Err(e) => Err(DbError::InternalError(format!("Lua error: {}", e))),
        };

        // Copy captured output back
        if let Ok(captured) = output_clone.lock() {
            output_capture.clear();
            output_capture.extend(captured.iter().cloned());
        }

        // Extract updated variables from global scope
        // Scan all globals and capture user-defined variables (excluding built-ins)
        let mut updated_vars = HashMap::new();

        // Built-in globals to skip (Lua standard library + solidb namespace)
        let skip_globals: std::collections::HashSet<&str> = [
            // Lua standard library
            "solidb",
            "string",
            "table",
            "math",
            "utf8",
            "bit32",
            "coroutine",
            // Lua built-in functions
            "print",
            "type",
            "tostring",
            "tonumber",
            "pairs",
            "ipairs",
            "next",
            "select",
            "error",
            "pcall",
            "xpcall",
            "assert",
            "rawget",
            "rawset",
            "rawequal",
            "rawlen",
            "setmetatable",
            "getmetatable",
            "collectgarbage",
            // Lua global variables
            "_G",
            "_VERSION",
            // SoliDB globals (should not be overwritten by user variables)
            "db",
            "request",
            "response",
            "time",
            // Removed for security (but check anyway)
            "os",
            "io",
            "debug",
            "package",
            "dofile",
            "load",
            "loadfile",
            "require",
        ]
        .iter()
        .cloned()
        .collect();

        // Iterate all globals and capture user-defined variables
        if let Ok(pairs) = globals
            .pairs::<String, LuaValue>()
            .collect::<Result<Vec<_>, _>>()
        {
            for (name, val) in pairs {
                // Skip built-ins and nil values
                if skip_globals.contains(name.as_str()) || matches!(val, LuaValue::Nil) {
                    continue;
                }
                // Skip functions (they're replayed from history instead)
                if matches!(val, LuaValue::Function(_)) {
                    continue;
                }
                // For SoliDB handles (collection handles), save metadata to recreate later
                if let LuaValue::Table(ref t) = val {
                    if t.get::<bool>("_solidb_handle").unwrap_or(false) {
                        // Save metadata for recreation: {_solidb_handle: true, _db: "...", _name: "..."}
                        let mut handle_meta = serde_json::Map::new();
                        handle_meta.insert("_solidb_handle".to_string(), JsonValue::Bool(true));
                        if let Ok(db_name) = t.get::<String>("_db") {
                            handle_meta.insert("_db".to_string(), JsonValue::String(db_name));
                        }
                        if let Ok(coll_name) = t.get::<String>("_name") {
                            handle_meta.insert("_name".to_string(), JsonValue::String(coll_name));
                        }
                        updated_vars.insert(name, JsonValue::Object(handle_meta));
                        continue;
                    }
                }
                // Convert to JSON and store
                if let Ok(json_val) = self.lua_to_json(&lua, val) {
                    updated_vars.insert(name, json_val);
                }
            }
        }

        match result {
            Ok(json_result) => Ok((json_result, updated_vars)),
            Err(e) => Err(e),
        }
    }

    /// Static helper for table_to_json used in closures
    fn table_to_json_static(lua: &Lua, table: mlua::Table) -> Result<JsonValue, mlua::Error> {
        let mut is_array = true;
        let mut expected_index = 1i64;

        for pair in table.clone().pairs::<LuaValue, LuaValue>() {
            let (k, _) = pair?;
            match k {
                LuaValue::Integer(i) if i == expected_index => {
                    expected_index += 1;
                }
                _ => {
                    is_array = false;
                    break;
                }
            }
        }

        if is_array && expected_index > 1 {
            let mut arr = Vec::new();
            for i in 1..expected_index {
                let val: LuaValue = table.get(i)?;
                arr.push(Self::lua_value_to_json_static(lua, val)?);
            }
            Ok(JsonValue::Array(arr))
        } else {
            let mut map = serde_json::Map::new();
            for pair in table.pairs::<LuaValue, LuaValue>() {
                let (k, v) = pair?;
                let key_str = match k {
                    LuaValue::String(s) => s.to_str()?.to_string(),
                    LuaValue::Integer(i) => i.to_string(),
                    LuaValue::Number(n) => n.to_string(),
                    _ => continue,
                };
                map.insert(key_str, Self::lua_value_to_json_static(lua, v)?);
            }
            Ok(JsonValue::Object(map))
        }
    }

    /// Static helper for lua_value to json conversion
    fn lua_value_to_json_static(lua: &Lua, value: LuaValue) -> Result<JsonValue, mlua::Error> {
        match value {
            LuaValue::Nil => Ok(JsonValue::Null),
            LuaValue::Boolean(b) => Ok(JsonValue::Bool(b)),
            LuaValue::Integer(i) => Ok(JsonValue::Number(i.into())),
            LuaValue::Number(n) => Ok(serde_json::Number::from_f64(n)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null)),
            LuaValue::String(s) => Ok(JsonValue::String(s.to_str()?.to_string())),
            LuaValue::Table(t) => Self::table_to_json_static(lua, t),
            _ => Ok(JsonValue::Null),
        }
    }

    fn setup_lua_globals(
        &self,
        lua: &Lua,
        db_name: &str,
        context: &ScriptContext,
        script_info: Option<(&str, &str)>,
    ) -> Result<(), DbError> {
        let globals = lua.globals();

        // Create 'solidb' namespace
        let solidb = lua
            .create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create solidb table: {}", e)))?;

        // solidb.log(msg)
        let storage_log = self.storage.clone();
        let db_log = db_name.to_string();
        let script_details = script_info.map(|(k, n)| (k.to_string(), n.to_string()));

        let log_fn = lua
            .create_function(move |lua, val: mlua::Value| {
                let msg = match val {
                    mlua::Value::String(ref s) => s.to_str()?.to_string(),
                    _ => {
                        let json_val = lua_to_json_value(lua, val)?;
                        serde_json::to_string(&json_val).map_err(mlua::Error::external)?
                    }
                };

                tracing::info!("[Lua Script] {}", msg);

                if let Some((sid, sname)) = &script_details {
                    if let Ok(db) = storage_log.get_database(&db_log) {
                        let collection_res = db.get_collection("_logs");
                        let collection = match collection_res {
                            Ok(c) => Some(c),
                            Err(DbError::CollectionNotFound(_)) => {
                                // Try to create it
                                if db.create_collection("_logs".to_string(), None).is_ok() {
                                    db.get_collection("_logs").ok()
                                } else {
                                    None
                                }
                            }
                            Err(_) => None,
                        };

                        if let Some(collection) = collection {
                            let timestamp = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as i64;

                            let log_entry = serde_json::json!({
                                "script_id": sid,
                                "script_name": sname,
                                "message": msg,
                                "timestamp": timestamp,
                                "level": "INFO"
                            });

                            let _ = collection.insert(log_entry);
                        }
                    }
                }
                Ok(())
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create log function: {}", e)))?;
        solidb
            .set("log", log_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set log: {}", e)))?;

        // solidb.stats() -> table
        let stats_ref = self.stats.clone();
        let stats_fn = lua
            .create_function(move |lua, (): ()| {
                let table = lua.create_table()?;
                table.set(
                    "active_scripts",
                    stats_ref.active_scripts.load(Ordering::SeqCst),
                )?;
                table.set("active_ws", stats_ref.active_ws.load(Ordering::SeqCst))?;
                table.set(
                    "total_scripts_executed",
                    stats_ref.total_scripts_executed.load(Ordering::SeqCst),
                )?;
                table.set(
                    "total_ws_connections",
                    stats_ref.total_ws_connections.load(Ordering::SeqCst),
                )?;
                Ok(table)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create stats function: {}", e))
            })?;
        solidb
            .set("stats", stats_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set stats: {}", e)))?;

        // solidb.now() -> Unix timestamp
        let now_fn = lua
            .create_function(|_, (): ()| {
                Ok(std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs())
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create now function: {}", e)))?;
        solidb
            .set("now", now_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set now: {}", e)))?;

        // Extend string library with regex
        if let Ok(string_table) = globals.get::<mlua::Table>("string") {
            // string.regex(subject, pattern) - Use safe_regex to prevent DoS
            let regex_fn = lua
                .create_function(|_, (s, pattern): (String, String)| {
                    let re = safe_regex(&pattern)
                        .map_err(|e| mlua::Error::RuntimeError(e))?;
                    Ok(re.is_match(&s))
                })
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to create regex function: {}", e))
                })?;

            string_table.set("regex", regex_fn).map_err(|e| {
                DbError::InternalError(format!("Failed to set string.regex: {}", e))
            })?;

            // string.regex_replace(subject, pattern, replacement) - Use safe_regex to prevent DoS
            let regex_replace_fn = lua
                .create_function(|_, (s, pattern, replacement): (String, String, String)| {
                    let re = safe_regex(&pattern)
                        .map_err(|e| mlua::Error::RuntimeError(e))?;
                    Ok(re.replace_all(&s, replacement.as_str()).to_string())
                })
                .map_err(|e| {
                    DbError::InternalError(format!(
                        "Failed to create regex_replace function: {}",
                        e
                    ))
                })?;

            string_table
                .set("regex_replace", regex_replace_fn)
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to set string.regex_replace: {}", e))
                })?;

            // string.slugify(text) - URL-friendly strings
            let slugify_fn = create_slugify_function(lua).map_err(|e| {
                DbError::InternalError(format!("Failed to create slugify function: {}", e))
            })?;
            string_table.set("slugify", slugify_fn).map_err(|e| {
                DbError::InternalError(format!("Failed to set string.slugify: {}", e))
            })?;

            // string.truncate(text, length, suffix) - Text truncation
            let truncate_fn = create_truncate_function(lua).map_err(|e| {
                DbError::InternalError(format!("Failed to create truncate function: {}", e))
            })?;
            string_table.set("truncate", truncate_fn).map_err(|e| {
                DbError::InternalError(format!("Failed to set string.truncate: {}", e))
            })?;

            // string.template(template, vars) - String interpolation with {{var}} syntax
            let template_fn = create_template_function(lua).map_err(|e| {
                DbError::InternalError(format!("Failed to create template function: {}", e))
            })?;
            string_table.set("template", template_fn).map_err(|e| {
                DbError::InternalError(format!("Failed to set string.template: {}", e))
            })?;

            // string.split(text, delimiter) - Split string into array
            let split_fn = create_split_function(lua).map_err(|e| {
                DbError::InternalError(format!("Failed to create split function: {}", e))
            })?;
            string_table.set("split", split_fn).map_err(|e| {
                DbError::InternalError(format!("Failed to set string.split: {}", e))
            })?;

            // string.trim(text) - Trim whitespace
            let trim_fn = create_trim_function(lua).map_err(|e| {
                DbError::InternalError(format!("Failed to create trim function: {}", e))
            })?;
            string_table
                .set("trim", trim_fn)
                .map_err(|e| DbError::InternalError(format!("Failed to set string.trim: {}", e)))?;

            // string.pad_left(text, length, char) - Left pad string
            let pad_left_fn = create_pad_left_function(lua).map_err(|e| {
                DbError::InternalError(format!("Failed to create pad_left function: {}", e))
            })?;
            string_table.set("pad_left", pad_left_fn).map_err(|e| {
                DbError::InternalError(format!("Failed to set string.pad_left: {}", e))
            })?;

            // string.pad_right(text, length, char) - Right pad string
            let pad_right_fn = create_pad_right_function(lua).map_err(|e| {
                DbError::InternalError(format!("Failed to create pad_right function: {}", e))
            })?;
            string_table.set("pad_right", pad_right_fn).map_err(|e| {
                DbError::InternalError(format!("Failed to set string.pad_right: {}", e))
            })?;

            // string.capitalize(text) - Capitalize first letter
            let capitalize_fn = create_capitalize_function(lua).map_err(|e| {
                DbError::InternalError(format!("Failed to create capitalize function: {}", e))
            })?;
            string_table.set("capitalize", capitalize_fn).map_err(|e| {
                DbError::InternalError(format!("Failed to set string.capitalize: {}", e))
            })?;

            // string.title_case(text) - Title case (capitalize each word)
            let title_case_fn = create_title_case_function(lua).map_err(|e| {
                DbError::InternalError(format!("Failed to create title_case function: {}", e))
            })?;
            string_table.set("title_case", title_case_fn).map_err(|e| {
                DbError::InternalError(format!("Failed to set string.title_case: {}", e))
            })?;
        }

        // Extend table library with utility functions
        if let Ok(table_lib) = globals.get::<mlua::Table>("table") {
            // table.deep_merge(t1, t2) - Recursive table merging
            let deep_merge_fn = create_deep_merge_function(lua).map_err(|e| {
                DbError::InternalError(format!("Failed to create deep_merge function: {}", e))
            })?;
            table_lib.set("deep_merge", deep_merge_fn).map_err(|e| {
                DbError::InternalError(format!("Failed to set table.deep_merge: {}", e))
            })?;

            // table.keys(t) - Get array of keys
            let keys_fn = create_keys_function(lua).map_err(|e| {
                DbError::InternalError(format!("Failed to create keys function: {}", e))
            })?;
            table_lib
                .set("keys", keys_fn)
                .map_err(|e| DbError::InternalError(format!("Failed to set table.keys: {}", e)))?;

            // table.values(t) - Get array of values
            let values_fn = create_values_function(lua).map_err(|e| {
                DbError::InternalError(format!("Failed to create values function: {}", e))
            })?;
            table_lib.set("values", values_fn).map_err(|e| {
                DbError::InternalError(format!("Failed to set table.values: {}", e))
            })?;

            // table.contains(t, value) - Check if table contains value
            let contains_fn = create_contains_function(lua).map_err(|e| {
                DbError::InternalError(format!("Failed to create contains function: {}", e))
            })?;
            table_lib.set("contains", contains_fn).map_err(|e| {
                DbError::InternalError(format!("Failed to set table.contains: {}", e))
            })?;

            // table.filter(t, predicate) - Filter table by predicate function
            let filter_fn = create_filter_function(lua).map_err(|e| {
                DbError::InternalError(format!("Failed to create filter function: {}", e))
            })?;
            table_lib.set("filter", filter_fn).map_err(|e| {
                DbError::InternalError(format!("Failed to set table.filter: {}", e))
            })?;

            // table.map(t, transform) - Transform table values
            let map_fn = create_map_function(lua).map_err(|e| {
                DbError::InternalError(format!("Failed to create map function: {}", e))
            })?;
            table_lib
                .set("map", map_fn)
                .map_err(|e| DbError::InternalError(format!("Failed to set table.map: {}", e)))?;
        }

        // solidb.fetch(url, options)
        let fetch_fn = lua
            .create_async_function(
                |lua, (url, options): (String, Option<LuaValue>)| async move {
                    let client = reqwest::Client::new();
                    let mut req_builder = client.get(&url); // Default to GET

                    if let Some(LuaValue::Table(t)) = options {
                        // Method
                        if let Ok(method) = t.get::<String>("method") {
                            match method.to_uppercase().as_str() {
                                "POST" => req_builder = client.post(&url),
                                "PUT" => req_builder = client.put(&url),
                                "DELETE" => req_builder = client.delete(&url),
                                "PATCH" => req_builder = client.patch(&url),
                                "HEAD" => req_builder = client.head(&url),
                                _ => {} // Default GET
                            }
                        }

                        // Headers
                        if let Ok(LuaValue::Table(h)) = t.get::<LuaValue>("headers") {
                            for (k, v) in h.pairs::<String, String>().flatten() {
                                req_builder = req_builder.header(k, v);
                            }
                        }

                        // Body
                        if let Ok(body) = t.get::<String>("body") {
                            req_builder = req_builder.body(body);
                        }
                    }

                    match req_builder.send().await {
                        Ok(res) => {
                            let status = res.status().as_u16();
                            let headers_map = res.headers().clone();
                            let text = res.text().await.unwrap_or_default();

                            let response_table = lua.create_table()?;
                            response_table.set("status", status)?;
                            response_table.set("body", text)?;
                            response_table.set("ok", (200..300).contains(&status))?;

                            let resp_headers = lua.create_table()?;
                            for (k, v) in headers_map.iter() {
                                if let Ok(val_str) = v.to_str() {
                                    resp_headers.set(k.as_str(), val_str)?;
                                }
                            }
                            response_table.set("headers", resp_headers)?;

                            Ok(response_table)
                        }
                        Err(e) => Err(mlua::Error::RuntimeError(format!("Fetch error: {}", e))),
                    }
                },
            )
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create fetch function: {}", e))
            })?;

        solidb
            .set("fetch", fetch_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set fetch: {}", e)))?;

        // solidb.json_encode(value) -> string
        let json_encode_fn = lua
            .create_function(|lua, val: LuaValue| {
                let json_val = lua_to_json_value(lua, val)?;
                serde_json::to_string(&json_val).map_err(mlua::Error::external)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create json_encode function: {}", e))
            })?;
        solidb
            .set("json_encode", json_encode_fn.clone())
            .map_err(|e| DbError::InternalError(format!("Failed to set json_encode: {}", e)))?;

        // solidb.json_decode(string) -> value
        let json_decode_fn = lua
            .create_function(|lua, s: String| {
                let json_val: JsonValue =
                    serde_json::from_str(&s).map_err(mlua::Error::external)?;
                json_to_lua(lua, &json_val)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create json_decode function: {}", e))
            })?;
        solidb
            .set("json_decode", json_decode_fn.clone())
            .map_err(|e| DbError::InternalError(format!("Failed to set json_decode: {}", e)))?;

        // Create global json table for convenience (json.encode / json.decode)
        let json_table = lua
            .create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create json table: {}", e)))?;
        json_table
            .set("encode", json_encode_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set json.encode: {}", e)))?;
        json_table
            .set("decode", json_decode_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set json.decode: {}", e)))?;
        globals
            .set("json", json_table)
            .map_err(|e| DbError::InternalError(format!("Failed to set global json: {}", e)))?;

        // Add validation functions to solidb namespace
        let validate_fn = create_validate_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create validate function: {}", e))
        })?;
        solidb
            .set("validate", validate_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set validate: {}", e)))?;

        let validate_detailed_fn = create_validate_detailed_function(lua).map_err(|e| {
            DbError::InternalError(format!(
                "Failed to create validate_detailed function: {}",
                e
            ))
        })?;
        solidb
            .set("validate_detailed", validate_detailed_fn)
            .map_err(|e| {
                DbError::InternalError(format!("Failed to set validate_detailed: {}", e))
            })?;

        let sanitize_fn = create_sanitize_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create sanitize function: {}", e))
        })?;
        solidb
            .set("sanitize", sanitize_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set sanitize: {}", e)))?;

        let typeof_fn = create_typeof_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create typeof function: {}", e))
        })?;
        solidb
            .set("typeof", typeof_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set typeof: {}", e)))?;

        // HTTP helpers
        let redirect_fn = create_redirect_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create redirect function: {}", e))
        })?;
        solidb
            .set("redirect", redirect_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set redirect: {}", e)))?;

        let set_cookie_fn = create_set_cookie_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create set_cookie function: {}", e))
        })?;
        solidb
            .set("set_cookie", set_cookie_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set set_cookie: {}", e)))?;

        let cache_fn = create_cache_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create cache function: {}", e))
        })?;
        solidb
            .set("cache", cache_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set cache: {}", e)))?;

        let cache_get_fn = create_cache_get_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create cache_get function: {}", e))
        })?;
        solidb
            .set("cache_get", cache_get_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set cache_get: {}", e)))?;

        // Error handling functions
        let error_fn = create_error_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create error function: {}", e))
        })?;
        solidb
            .set("error", error_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set error: {}", e)))?;

        let dev_assert_fn = create_dev_assert_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create dev_assert function: {}", e))
        })?;
        solidb
            .set("assert", dev_assert_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set assert: {}", e)))?;

        let try_fn = create_try_function(lua)
            .map_err(|e| DbError::InternalError(format!("Failed to create try function: {}", e)))?;
        solidb
            .set("try", try_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set try: {}", e)))?;

        let validate_condition_fn = create_validate_condition_function(lua).map_err(|e| {
            DbError::InternalError(format!(
                "Failed to create validate_condition function: {}",
                e
            ))
        })?;
        solidb
            .set("validate_condition", validate_condition_fn)
            .map_err(|e| {
                DbError::InternalError(format!("Failed to set validate_condition: {}", e))
            })?;

        let check_permissions_fn = create_check_permissions_function(lua).map_err(|e| {
            DbError::InternalError(format!(
                "Failed to create check_permissions function: {}",
                e
            ))
        })?;
        solidb
            .set("check_permissions", check_permissions_fn)
            .map_err(|e| {
                DbError::InternalError(format!("Failed to set check_permissions: {}", e))
            })?;

        let validate_input_fn = create_validate_input_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create validate_input function: {}", e))
        })?;
        solidb
            .set("validate_input", validate_input_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set validate_input: {}", e)))?;

        let rate_limit_fn = create_rate_limit_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create rate_limit function: {}", e))
        })?;
        solidb
            .set("rate_limit", rate_limit_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set rate_limit: {}", e)))?;

        let timeout_fn = create_timeout_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create timeout function: {}", e))
        })?;
        solidb
            .set("timeout", timeout_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set timeout: {}", e)))?;

        let retry_fn = create_retry_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create retry function: {}", e))
        })?;
        solidb
            .set("retry", retry_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set retry: {}", e)))?;

        let fallback_fn = create_fallback_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create fallback function: {}", e))
        })?;
        solidb
            .set("fallback", fallback_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set fallback: {}", e)))?;

        // Authentication & Authorization (solidb.auth namespace)
        let auth_table = auth::create_auth_table(lua, &context.user)
            .map_err(|e| DbError::InternalError(format!("Failed to create auth table: {}", e)))?;
        solidb
            .set("auth", auth_table)
            .map_err(|e| DbError::InternalError(format!("Failed to set auth: {}", e)))?;

        // File & Media Handling (using blob storage)
        let upload_fn = create_upload_function(lua, self.storage.clone(), db_name.to_string())
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create upload function: {}", e))
            })?;
        solidb
            .set("upload", upload_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set upload: {}", e)))?;

        let file_info_fn =
            create_file_info_function(lua, self.storage.clone(), db_name.to_string()).map_err(
                |e| DbError::InternalError(format!("Failed to create file_info function: {}", e)),
            )?;
        solidb
            .set("file_info", file_info_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set file_info: {}", e)))?;

        let file_read_fn =
            create_file_read_function(lua, self.storage.clone(), db_name.to_string()).map_err(
                |e| DbError::InternalError(format!("Failed to create file_read function: {}", e)),
            )?;
        solidb
            .set("file_read", file_read_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set file_read: {}", e)))?;

        let file_delete_fn =
            create_file_delete_function(lua, self.storage.clone(), db_name.to_string()).map_err(
                |e| DbError::InternalError(format!("Failed to create file_delete function: {}", e)),
            )?;
        solidb
            .set("file_delete", file_delete_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set file_delete: {}", e)))?;

        let file_list_fn =
            create_file_list_function(lua, self.storage.clone(), db_name.to_string()).map_err(
                |e| DbError::InternalError(format!("Failed to create file_list function: {}", e)),
            )?;
        solidb
            .set("file_list", file_list_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set file_list: {}", e)))?;

        let image_process_fn = create_image_process_function(
            lua,
            self.storage.clone(),
            db_name.to_string(),
        )
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create image_process function: {}", e))
        })?;
        solidb
            .set("image_process", image_process_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set image_process: {}", e)))?;

        // Development Tools
        let debug_fn = create_debug_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create debug function: {}", e))
        })?;
        solidb
            .set("debug", debug_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set debug: {}", e)))?;

        let inspect_fn = create_inspect_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create inspect function: {}", e))
        })?;
        solidb
            .set("inspect", inspect_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set inspect: {}", e)))?;

        let profile_fn = create_profile_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create profile function: {}", e))
        })?;
        solidb
            .set("profile", profile_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set profile: {}", e)))?;

        let benchmark_fn = create_benchmark_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create benchmark function: {}", e))
        })?;
        solidb
            .set("benchmark", benchmark_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set benchmark: {}", e)))?;

        let mock_fn = create_mock_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create mock function: {}", e))
        })?;
        solidb
            .set("mock", mock_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set mock: {}", e)))?;

        let dev_assert_fn = create_dev_assert_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create dev_assert function: {}", e))
        })?;
        solidb
            .set("assert", dev_assert_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set assert: {}", e)))?;

        let assert_eq_fn = create_assert_eq_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create assert_eq function: {}", e))
        })?;
        solidb
            .set("assert_eq", assert_eq_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set assert_eq: {}", e)))?;

        let dump_fn = create_dump_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create dump function: {}", e))
        })?;
        solidb
            .set("dump", dump_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set dump: {}", e)))?;

        // Set solidb global
        // Initialize solidb.env table
        let env_table = lua
            .create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create env table: {}", e)))?;

        // Populate env table from _env collection
        if let Ok(db) = self.storage.get_database(&db_name) {
            if let Ok(collection) = db.get_collection("_env") {
                let collection: &crate::storage::Collection = &collection;
                let all_docs = collection.scan(None);
                for doc in all_docs {
                    if let (Some(key), Some(value)) = (
                        doc.get("_key")
                            .and_then(|v| v.as_str().map(|s| s.to_string())),
                        doc.get("value")
                            .and_then(|v| v.as_str().map(|s| s.to_string())),
                    ) {
                        env_table.set(key, value).map_err(|e| {
                            DbError::InternalError(format!("Failed to set env var: {}", e))
                        })?;
                    }
                }
            }
        }

        // Create 'streams' module
        if let Some(stream_manager) = self.stream_manager.clone() {
            let streams_table = lua.create_table().map_err(|e| DbError::InternalError(format!("Failed to create streams table: {}", e)))?;

            // solidb.streams.list() -> array of {name: string, query: string, created_at: number}
            let manager_list = stream_manager.clone();
            let list_fn = lua.create_function(move |lua, (): ()| {
                let streams = manager_list.list_streams();
                let mut result = Vec::new();
                for stream in streams {
                    let mut s = serde_json::Map::new();
                    s.insert("name".to_string(), serde_json::Value::String(stream.name));
                    // We might not want to expose full complex query object, maybe just source collection?
                    // Or string representation if we had it.
                    // For now, let's just expose created_at
                    s.insert("created_at".to_string(), serde_json::Value::Number(serde_json::Number::from(stream.created_at)));
                    result.push(serde_json::Value::Object(s));
                }
                
                // Use the json helper to convert to Lua table
                json_to_lua(lua, &serde_json::Value::Array(result))
            }).map_err(|e| DbError::InternalError(format!("Failed to create streams.list: {}", e)))?;
            
            streams_table.set("list", list_fn).map_err(|e| DbError::InternalError(format!("Failed to set streams.list: {}", e)))?;

            // solidb.streams.stop(name) -> void
            let manager_stop = stream_manager.clone();
            let stop_fn = lua.create_function(move |_, name: String| {
                manager_stop.stop_stream(&name).map_err(|e| mlua::Error::RuntimeError(e.to_string()))
            }).map_err(|e| DbError::InternalError(format!("Failed to create streams.stop: {}", e)))?;
            
            streams_table.set("stop", stop_fn).map_err(|e| DbError::InternalError(format!("Failed to set streams.stop: {}", e)))?;
            
            solidb.set("streams", streams_table).map_err(|e| DbError::InternalError(format!("Failed to set solidb.streams: {}", e)))?;
        }

        solidb
            .set("env", env_table)
            .map_err(|e| DbError::InternalError(format!("Failed to set solidb.env: {}", e)))?;

        // Add AI bindings (solidb.ai.*)
        let ai_table = ai_bindings::create_ai_table(&lua, self.storage.clone(), db_name)
            .map_err(|e| DbError::InternalError(format!("Failed to create AI table: {}", e)))?;
        solidb
            .set("ai", ai_table)
            .map_err(|e| DbError::InternalError(format!("Failed to set solidb.ai: {}", e)))?;

        globals
            .set("solidb", solidb)
            .map_err(|e| DbError::InternalError(format!("Failed to set solidb global: {}", e)))?;

        // Create 'time' module with safe time functions
        let time_table = lua
            .create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create time table: {}", e)))?;

        // time.now() - current Unix timestamp in seconds
        let now_fn = lua
            .create_function(|_, ()| {
                use std::time::{SystemTime, UNIX_EPOCH};
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(mlua::Error::external)?;
                Ok(now.as_secs() as i64)
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create time.now: {}", e)))?;
        time_table
            .set("now", now_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.now: {}", e)))?;

        // time.millis() - current Unix timestamp in milliseconds
        let millis_fn = lua
            .create_function(|_, ()| {
                use std::time::{SystemTime, UNIX_EPOCH};
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(mlua::Error::external)?;
                Ok(now.as_millis() as i64)
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create time.millis: {}", e)))?;
        time_table
            .set("millis", millis_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.millis: {}", e)))?;

        // time.date(format, timestamp?) - format a timestamp (or current time)
        let date_fn = lua
            .create_function(|_, (format, timestamp): (Option<String>, Option<i64>)| {
                use chrono::{DateTime, TimeZone, Utc};
                let fmt = format.unwrap_or_else(|| "%Y-%m-%d %H:%M:%S".to_string());
                let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                    Utc.timestamp_opt(ts, 0)
                        .single()
                        .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
                } else {
                    Utc::now()
                };
                Ok(dt.format(&fmt).to_string())
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create time.date: {}", e)))?;
        time_table
            .set("date", date_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.date: {}", e)))?;

        // time.parse(str, format?) - parse a date string to timestamp
        let parse_fn = lua
            .create_function(|_, (date_str, format): (String, Option<String>)| {
                use chrono::{DateTime, NaiveDateTime, Utc};
                let fmt = format.unwrap_or_else(|| "%Y-%m-%d %H:%M:%S".to_string());
                let naive = NaiveDateTime::parse_from_str(&date_str, &fmt)
                    .map_err(|e| mlua::Error::external(format!("Date parse error: {}", e)))?;
                let dt: DateTime<Utc> = DateTime::from_naive_utc_and_offset(naive, Utc);
                Ok(dt.timestamp())
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create time.parse: {}", e)))?;
        time_table
            .set("parse", parse_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.parse: {}", e)))?;

        // time.iso(timestamp?) - format as ISO 8601 string
        let iso_fn = lua
            .create_function(|_, timestamp: Option<i64>| {
                use chrono::{DateTime, TimeZone, Utc};
                let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                    Utc.timestamp_opt(ts, 0)
                        .single()
                        .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
                } else {
                    Utc::now()
                };
                Ok(dt.to_rfc3339())
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create time.iso: {}", e)))?;
        time_table
            .set("iso", iso_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.iso: {}", e)))?;

        // time.diff(t1, t2) - difference in seconds
        let diff_fn = lua
            .create_function(|_, (t1, t2): (i64, i64)| Ok(t1 - t2))
            .map_err(|e| DbError::InternalError(format!("Failed to create time.diff: {}", e)))?;
        time_table
            .set("diff", diff_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.diff: {}", e)))?;

        // time.add(timestamp, seconds) - add seconds to timestamp
        let add_fn = lua
            .create_function(|_, (timestamp, seconds): (i64, i64)| Ok(timestamp + seconds))
            .map_err(|e| DbError::InternalError(format!("Failed to create time.add: {}", e)))?;
        time_table
            .set("add", add_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.add: {}", e)))?;

        // time.year/month/day/hour/minute/second(timestamp?) - extract components
        let year_fn = lua
            .create_function(|_, timestamp: Option<i64>| {
                use chrono::{DateTime, Datelike, TimeZone, Utc};
                let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                    Utc.timestamp_opt(ts, 0)
                        .single()
                        .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
                } else {
                    Utc::now()
                };
                Ok(dt.year())
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create time.year: {}", e)))?;
        time_table
            .set("year", year_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.year: {}", e)))?;

        let month_fn = lua
            .create_function(|_, timestamp: Option<i64>| {
                use chrono::{DateTime, Datelike, TimeZone, Utc};
                let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                    Utc.timestamp_opt(ts, 0)
                        .single()
                        .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
                } else {
                    Utc::now()
                };
                Ok(dt.month() as i32)
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create time.month: {}", e)))?;
        time_table
            .set("month", month_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.month: {}", e)))?;

        let day_fn = lua
            .create_function(|_, timestamp: Option<i64>| {
                use chrono::{DateTime, Datelike, TimeZone, Utc};
                let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                    Utc.timestamp_opt(ts, 0)
                        .single()
                        .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
                } else {
                    Utc::now()
                };
                Ok(dt.day() as i32)
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create time.day: {}", e)))?;
        time_table
            .set("day", day_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.day: {}", e)))?;

        let hour_fn = lua
            .create_function(|_, timestamp: Option<i64>| {
                use chrono::{DateTime, TimeZone, Timelike, Utc};
                let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                    Utc.timestamp_opt(ts, 0)
                        .single()
                        .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
                } else {
                    Utc::now()
                };
                Ok(dt.hour() as i32)
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create time.hour: {}", e)))?;
        time_table
            .set("hour", hour_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.hour: {}", e)))?;

        let minute_fn = lua
            .create_function(|_, timestamp: Option<i64>| {
                use chrono::{DateTime, TimeZone, Timelike, Utc};
                let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                    Utc.timestamp_opt(ts, 0)
                        .single()
                        .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
                } else {
                    Utc::now()
                };
                Ok(dt.minute() as i32)
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create time.minute: {}", e)))?;
        time_table
            .set("minute", minute_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.minute: {}", e)))?;

        let second_fn = lua
            .create_function(|_, timestamp: Option<i64>| {
                use chrono::{DateTime, TimeZone, Timelike, Utc};
                let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                    Utc.timestamp_opt(ts, 0)
                        .single()
                        .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
                } else {
                    Utc::now()
                };
                Ok(dt.second() as i32)
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create time.second: {}", e)))?;
        time_table
            .set("second", second_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.second: {}", e)))?;

        // time.weekday(timestamp?) - day of week (1=Monday, 7=Sunday)
        let weekday_fn = lua
            .create_function(|_, timestamp: Option<i64>| {
                use chrono::{DateTime, Datelike, TimeZone, Utc};
                let dt: DateTime<Utc> = if let Some(ts) = timestamp {
                    Utc.timestamp_opt(ts, 0)
                        .single()
                        .ok_or_else(|| mlua::Error::external("Invalid timestamp"))?
                } else {
                    Utc::now()
                };
                Ok(dt.weekday().num_days_from_monday() as i32 + 1)
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create time.weekday: {}", e)))?;
        time_table
            .set("weekday", weekday_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.weekday: {}", e)))?;

        globals
            .set("time", time_table)
            .map_err(|e| DbError::InternalError(format!("Failed to set time global: {}", e)))?;

        // Add enhanced table functions (extend standard table library)
        lua.load(
            r#"
            -- table.sorted(t, comp?) - returns sorted copy of table
            function table.sorted(t, comp)
                local copy = {}
                for i, v in ipairs(t) do copy[i] = v end
                table.sort(copy, comp)
                return copy
            end

            -- table.keys(t) - returns array of keys
            function table.keys(t)
                local keys = {}
                for k, _ in pairs(t) do
                    keys[#keys + 1] = k
                end
                return keys
            end

            -- table.values(t) - returns array of values
            function table.values(t)
                local values = {}
                for _, v in pairs(t) do
                    values[#values + 1] = v
                end
                return values
            end

            -- table.merge(t1, t2) - merge two tables (t2 overwrites t1)
            function table.merge(t1, t2)
                local result = {}
                for k, v in pairs(t1) do result[k] = v end
                for k, v in pairs(t2) do result[k] = v end
                return result
            end

            -- table.filter(t, fn) - filter array elements
            function table.filter(t, fn)
                local result = {}
                for i, v in ipairs(t) do
                    if fn(v, i) then
                        result[#result + 1] = v
                    end
                end
                return result
            end

            -- table.map(t, fn) - map array elements
            function table.map(t, fn)
                local result = {}
                for i, v in ipairs(t) do
                    result[i] = fn(v, i)
                end
                return result
            end

            -- table.find(t, fn) - find first element matching predicate
            function table.find(t, fn)
                for i, v in ipairs(t) do
                    if fn(v, i) then
                        return v, i
                    end
                end
                return nil
            end

            -- table.contains(t, value) - check if array contains value
            function table.contains(t, value)
                for _, v in ipairs(t) do
                    if v == value then return true end
                end
                return false
            end

            -- table.reverse(t) - reverse array
            function table.reverse(t)
                local result = {}
                for i = #t, 1, -1 do
                    result[#result + 1] = t[i]
                end
                return result
            end

            -- table.slice(t, start, stop) - slice array
            function table.slice(t, start, stop)
                local result = {}
                start = start or 1
                stop = stop or #t
                for i = start, stop do
                    result[#result + 1] = t[i]
                end
                return result
            end

            -- table.len(t) - count elements (works for non-arrays too)
            function table.len(t)
                local count = 0
                for _ in pairs(t) do count = count + 1 end
                return count
            end
        "#,
        )
        .exec()
        .map_err(|e| DbError::InternalError(format!("Failed to setup table extensions: {}", e)))?;

        // Create global 'db' object
        let db_handle = lua
            .create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create db table: {}", e)))?;
        db_handle
            .set("_name", db_name.to_string())
            .map_err(|e| DbError::InternalError(format!("Failed to set db name: {}", e)))?;

        // db:collection(name) -> collection handle
        let storage_ref = self.storage.clone();
        let current_db = db_name.to_string();

        let collection_fn = lua
            .create_function(move |lua, (_, coll_name): (LuaValue, String)| {
                let storage = storage_ref.clone();
                let db_name = current_db.clone();

                // Create collection handle table
                let coll_handle = lua.create_table()?;
                coll_handle.set("_solidb_handle", true)?; // Marker to skip during session capture
                coll_handle.set("_db", db_name.clone())?;
                coll_handle.set("_name", coll_name.clone())?;

                // col:get(key)
                let storage_get = storage.clone();
                let db_get = db_name.clone();
                let coll_get = coll_name.clone();
                let get_fn = lua.create_function(move |lua, (_, key): (LuaValue, String)| {
                    let db = storage_get
                        .get_database(&db_get)
                        .map_err(mlua::Error::external)?;
                    let collection = db
                        .get_collection(&coll_get)
                        .map_err(mlua::Error::external)?;

                    match collection.get(&key) {
                        Ok(doc) => {
                            let json_val = doc.to_value();
                            json_to_lua(lua, &json_val)
                        }
                        Err(DbError::DocumentNotFound(_)) => Ok(LuaValue::Nil),
                        Err(e) => Err(mlua::Error::external(e)),
                    }
                })?;
                coll_handle.set("get", get_fn)?;

                // col:insert(doc)
                let storage_insert = storage.clone();
                let db_insert = db_name.clone();
                let coll_insert = coll_name.clone();
                let insert_fn =
                    lua.create_function(move |lua, (_, doc): (LuaValue, LuaValue)| {
                        let json_doc = lua_to_json_value(lua, doc)?;

                        let db = storage_insert
                            .get_database(&db_insert)
                            .map_err(mlua::Error::external)?;
                        let collection = db
                            .get_collection(&coll_insert)
                            .map_err(mlua::Error::external)?;

                        let inserted = collection
                            .insert(json_doc)
                            .map_err(mlua::Error::external)?;

                        json_to_lua(lua, &inserted.to_value())
                    })?;
                coll_handle.set("insert", insert_fn)?;

                // col:update(key, doc)
                let storage_update = storage.clone();
                let db_update = db_name.clone();
                let coll_update = coll_name.clone();
                let update_fn = lua.create_function(
                    move |lua, (_, key, doc): (LuaValue, String, LuaValue)| {
                        let json_doc = lua_to_json_value(lua, doc)?;

                        let db = storage_update
                            .get_database(&db_update)
                            .map_err(mlua::Error::external)?;
                        let collection = db
                            .get_collection(&coll_update)
                            .map_err(mlua::Error::external)?;

                        let updated = collection
                            .update(&key, json_doc)
                            .map_err(mlua::Error::external)?;

                        json_to_lua(lua, &updated.to_value())
                    },
                )?;
                coll_handle.set("update", update_fn)?;

                // col:delete(key)
                let storage_delete = storage.clone();
                let db_delete = db_name.clone();
                let coll_delete = coll_name.clone();
                let delete_fn = lua.create_function(move |_, (_, key): (LuaValue, String)| {
                    let db = storage_delete
                        .get_database(&db_delete)
                        .map_err(mlua::Error::external)?;
                    let collection = db
                        .get_collection(&coll_delete)
                        .map_err(mlua::Error::external)?;

                    collection
                        .delete(&key)
                        .map_err(mlua::Error::external)?;

                    Ok(true)
                })?;
                coll_handle.set("delete", delete_fn)?;

                // col:count(filter?) - count all or matching documents
                let storage_count = storage.clone();
                let db_count = db_name.clone();
                let coll_count = coll_name.clone();
                let count_fn =
                    lua.create_function(move |lua, (_, filter): (LuaValue, Option<LuaValue>)| {
                        let db = storage_count
                            .get_database(&db_count)
                            .map_err(mlua::Error::external)?;
                        let collection = db
                            .get_collection(&coll_count)
                            .map_err(mlua::Error::external)?;

                        match filter {
                            Some(f) if !matches!(f, LuaValue::Nil) => {
                                let filter_json = lua_to_json_value(lua, f)?;
                                // Count matching documents
                                let all_docs = collection.scan(None);
                                let count = all_docs
                                    .into_iter()
                                    .filter(|doc| matches_filter(&doc.to_value(), &filter_json))
                                    .count();
                                Ok(count as i64)
                            }
                            _ => Ok(collection.count() as i64),
                        }
                    })?;
                coll_handle.set("count", count_fn)?;

                // col:find(filter) - find documents matching filter
                let storage_find = storage.clone();
                let db_find = db_name.clone();
                let coll_find = coll_name.clone();
                let find_fn =
                    lua.create_function(move |lua, (_, filter): (LuaValue, LuaValue)| {
                        let filter_json = lua_to_json_value(lua, filter)?;

                        let db = storage_find
                            .get_database(&db_find)
                            .map_err(mlua::Error::external)?;
                        let collection = db
                            .get_collection(&coll_find)
                            .map_err(mlua::Error::external)?;

                        // Scan all documents and filter
                        let all_docs = collection.scan(None);
                        let mut results = Vec::new();

                        for doc in all_docs {
                            let doc_value = doc.to_value();
                            if matches_filter(&doc_value, &filter_json) {
                                results.push(doc_value);
                            }
                        }

                        // Convert to Lua table
                        let result_table = lua.create_table()?;
                        for (i, doc) in results.iter().enumerate() {
                            result_table.set(i + 1, json_to_lua(lua, doc)?)?;
                        }

                        Ok(LuaValue::Table(result_table))
                    })?;
                coll_handle.set("find", find_fn)?;

                // col:find_one(filter) - find first document matching filter
                let storage_find_one = storage.clone();
                let db_find_one = db_name.clone();
                let coll_find_one = coll_name.clone();
                let find_one_fn =
                    lua.create_function(move |lua, (_, filter): (LuaValue, LuaValue)| {
                        let filter_json = lua_to_json_value(lua, filter)?;

                        let db = storage_find_one
                            .get_database(&db_find_one)
                            .map_err(mlua::Error::external)?;
                        let collection = db
                            .get_collection(&coll_find_one)
                            .map_err(mlua::Error::external)?;

                        // Scan documents and return first match
                        let all_docs = collection.scan(None);

                        for doc in all_docs {
                            let doc_value = doc.to_value();
                            if matches_filter(&doc_value, &filter_json) {
                                return json_to_lua(lua, &doc_value);
                            }
                        }

                        Ok(LuaValue::Nil)
                    })?;
                coll_handle.set("find_one", find_one_fn)?;

                // col:bulk_insert(docs) - insert multiple documents
                let storage_bulk = storage.clone();
                let db_bulk = db_name.clone();
                let coll_bulk = coll_name.clone();
                let bulk_insert_fn =
                    lua.create_function(move |lua, (_, docs): (LuaValue, LuaValue)| {
                        let docs_json = lua_to_json_value(lua, docs)?;

                        let db = storage_bulk
                            .get_database(&db_bulk)
                            .map_err(mlua::Error::external)?;
                        let collection = db
                            .get_collection(&coll_bulk)
                            .map_err(mlua::Error::external)?;

                        let docs_array = match docs_json {
                            JsonValue::Array(arr) => arr,
                            _ => {
                                return Err(mlua::Error::external(DbError::BadRequest(
                                    "bulk_insert expects an array of documents".to_string(),
                                )))
                            }
                        };

                        let mut inserted = Vec::new();
                        for doc in docs_array {
                            let result = collection
                                .insert(doc)
                                .map_err(mlua::Error::external)?;
                            inserted.push(result.to_value());
                        }

                        // Return array of inserted documents
                        let result_table = lua.create_table()?;
                        for (i, doc) in inserted.iter().enumerate() {
                            result_table.set(i + 1, json_to_lua(lua, doc)?)?;
                        }

                        Ok(LuaValue::Table(result_table))
                    })?;
                coll_handle.set("bulk_insert", bulk_insert_fn)?;

                // col:upsert(key_or_filter, doc) - insert or update
                // If key_or_filter is a string, it's treated as a _key lookup
                // If key_or_filter is a table, it's treated as a filter
                let storage_upsert = storage.clone();
                let db_upsert = db_name.clone();
                let coll_upsert = coll_name.clone();
                let upsert_fn = lua.create_function(
                    move |lua, (_, key_or_filter, doc): (LuaValue, LuaValue, LuaValue)| {
                        let mut doc_json = lua_to_json_value(lua, doc)?;

                        let db = storage_upsert
                            .get_database(&db_upsert)
                            .map_err(mlua::Error::external)?;
                        let collection = db
                            .get_collection(&coll_upsert)
                            .map_err(mlua::Error::external)?;

                        // Check if key_or_filter is a string (key) or table (filter)
                        let existing_key: Option<String> = match &key_or_filter {
                            LuaValue::String(s) => {
                                let key = s.to_str()?.to_string();
                                // Check if document with this key exists
                                match collection.get(&key) {
                                    Ok(_) => Some(key),
                                    Err(_) => {
                                        // Set _key in doc for insert
                                        if let JsonValue::Object(ref mut obj) = doc_json {
                                            obj.insert(
                                                "_key".to_string(),
                                                JsonValue::String(key.clone()),
                                            );
                                        }
                                        None
                                    }
                                }
                            }
                            LuaValue::Table(_) => {
                                let filter_json = lua_to_json_value(lua, key_or_filter)?;
                                // Find existing document by filter
                                let all_docs = collection.scan(None);
                                let mut found_key = None;
                                for existing_doc in all_docs {
                                    let doc_value = existing_doc.to_value();
                                    if matches_filter(&doc_value, &filter_json) {
                                        if let Some(key) =
                                            doc_value.get("_key").and_then(|k| k.as_str())
                                        {
                                            found_key = Some(key.to_string());
                                            break;
                                        }
                                    }
                                }
                                found_key
                            }
                            _ => None,
                        };

                        let result = if let Some(key) = existing_key {
                            // Update existing
                            collection
                                .update(&key, doc_json)
                                .map_err(mlua::Error::external)?
                                .to_value()
                        } else {
                            // Insert new
                            collection
                                .insert(doc_json)
                                .map_err(mlua::Error::external)?
                                .to_value()
                        };

                        json_to_lua(lua, &result)
                    },
                )?;
                coll_handle.set("upsert", upsert_fn)?;

                Ok(LuaValue::Table(coll_handle))
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create collection function: {}", e))
            })?;

        db_handle.set("collection", collection_fn).map_err(|e| {
            DbError::InternalError(format!("Failed to set collection function: {}", e))
        })?;

        // db:query(query, bind_vars) -> results
        let storage_query = self.storage.clone();
        let db_query = db_name.to_string();
        let query_fn = lua
            .create_function(
                move |lua, (_, query, bind_vars): (LuaValue, String, Option<LuaValue>)| {
                    let storage = storage_query.clone();

                    // Parse bind vars
                    let bind_vars_map = if let Some(vars) = bind_vars {
                        let json_vars = lua_to_json_value(lua, vars)?;
                        if let JsonValue::Object(map) = json_vars {
                            map.into_iter().collect()
                        } else {
                            std::collections::HashMap::new()
                        }
                    } else {
                        std::collections::HashMap::new()
                    };

                    // Parse and execute query
                    let query_ast = parse(&query)
                        .map_err(|e| mlua::Error::external(DbError::BadRequest(e.to_string())))?;

                    let executor = if bind_vars_map.is_empty() {
                        QueryExecutor::with_database(&storage, db_query.clone())
                    } else {
                        QueryExecutor::with_database_and_bind_vars(
                            &storage,
                            db_query.clone(),
                            bind_vars_map,
                        )
                    };

                    let results = executor
                        .execute(&query_ast)
                        .map_err(mlua::Error::external)?;

                    // Convert results to Lua table
                    let result_table = lua.create_table()?;
                    for (i, doc) in results.iter().enumerate() {
                        result_table.set(i + 1, json_to_lua(lua, doc)?)?;
                    }

                    Ok(LuaValue::Table(result_table))
                },
            )
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create query function: {}", e))
            })?;

        db_handle
            .set("query", query_fn.clone())
            .map_err(|e| DbError::InternalError(format!("Failed to set query function: {}", e)))?;

        // db:transaction(callback) -> auto-commit/rollback transaction
        let storage_tx = self.storage.clone();
        let db_tx = db_name.to_string();
        let transaction_fn = lua
            .create_async_function(move |lua, (_, callback): (LuaValue, mlua::Function)| {
                let storage = storage_tx.clone();
                let db_name = db_tx.clone();

                async move {
                    // Initialize transaction manager if needed
                    storage
                        .initialize_transactions()
                        .map_err(mlua::Error::external)?;

                    // Get transaction manager and begin transaction
                    let tx_manager = storage
                        .transaction_manager()
                        .map_err(mlua::Error::external)?;

                    let tx_id = tx_manager
                        .begin(crate::transaction::IsolationLevel::ReadCommitted)
                        .map_err(mlua::Error::external)?;

                    // Create the transaction context table
                    let tx_handle = lua.create_table()?;
                    tx_handle.set("_tx_id", tx_id.to_string())?;
                    tx_handle.set("_db", db_name.clone())?;

                    // tx:collection(name) -> transactional collection handle
                    let storage_coll = storage.clone();
                    let tx_manager_coll = tx_manager.clone();
                    let db_coll = db_name.clone();
                    let tx_id_coll = tx_id;

                    let tx_collection_fn =
                        lua.create_function(move |lua, (_, coll_name): (LuaValue, String)| {
                            let storage = storage_coll.clone();
                            let tx_manager = tx_manager_coll.clone();
                            let db_name = db_coll.clone();
                            let tx_id = tx_id_coll;

                            // Create transactional collection handle
                            let coll_handle = lua.create_table()?;
                            coll_handle.set("_db", db_name.clone())?;
                            coll_handle.set("_name", coll_name.clone())?;
                            coll_handle.set("_tx_id", tx_id.to_string())?;

                            // col:insert(doc) - transactional insert
                            let storage_insert = storage.clone();
                            let tx_mgr_insert = tx_manager.clone();
                            let db_insert = db_name.clone();
                            let coll_insert = coll_name.clone();
                            let tx_id_insert = tx_id;
                            let insert_fn =
                                lua.create_function(move |lua, (_, doc): (LuaValue, LuaValue)| {
                                    let json_doc = lua_to_json_value(lua, doc)?;

                                    let full_coll_name = format!("{}:{}", db_insert, coll_insert);
                                    let collection = storage_insert
                                        .get_collection(&full_coll_name)
                                        .map_err(mlua::Error::external)?;

                                    let tx_arc = tx_mgr_insert
                                        .get(tx_id_insert)
                                        .map_err(mlua::Error::external)?;
                                    let mut tx = tx_arc.write().unwrap();
                                    let wal = tx_mgr_insert.wal();

                                    let inserted = collection
                                        .insert_tx(&mut tx, wal, json_doc)
                                        .map_err(mlua::Error::external)?;

                                    json_to_lua(lua, &inserted.to_value())
                                })?;
                            coll_handle.set("insert", insert_fn)?;

                            // col:update(key, doc) - transactional update
                            let storage_update = storage.clone();
                            let tx_mgr_update = tx_manager.clone();
                            let db_update = db_name.clone();
                            let coll_update = coll_name.clone();
                            let tx_id_update = tx_id;
                            let update_fn = lua.create_function(
                                move |lua, (_, key, doc): (LuaValue, String, LuaValue)| {
                                    let json_doc = lua_to_json_value(lua, doc)?;

                                    let full_coll_name = format!("{}:{}", db_update, coll_update);
                                    let collection = storage_update
                                        .get_collection(&full_coll_name)
                                        .map_err(mlua::Error::external)?;

                                    let tx_arc = tx_mgr_update
                                        .get(tx_id_update)
                                        .map_err(mlua::Error::external)?;
                                    let mut tx = tx_arc.write().unwrap();
                                    let wal = tx_mgr_update.wal();

                                    let updated = collection
                                        .update_tx(&mut tx, wal, &key, json_doc)
                                        .map_err(mlua::Error::external)?;

                                    json_to_lua(lua, &updated.to_value())
                                },
                            )?;
                            coll_handle.set("update", update_fn)?;

                            // col:delete(key) - transactional delete
                            let storage_delete = storage.clone();
                            let tx_mgr_delete = tx_manager.clone();
                            let db_delete = db_name.clone();
                            let coll_delete = coll_name.clone();
                            let tx_id_delete = tx_id;
                            let delete_fn =
                                lua.create_function(move |_, (_, key): (LuaValue, String)| {
                                    let full_coll_name = format!("{}:{}", db_delete, coll_delete);
                                    let collection = storage_delete
                                        .get_collection(&full_coll_name)
                                        .map_err(mlua::Error::external)?;

                                    let tx_arc = tx_mgr_delete
                                        .get(tx_id_delete)
                                        .map_err(mlua::Error::external)?;
                                    let mut tx = tx_arc.write().unwrap();
                                    let wal = tx_mgr_delete.wal();

                                    collection
                                        .delete_tx(&mut tx, wal, &key)
                                        .map_err(mlua::Error::external)?;

                                    Ok(true)
                                })?;
                            coll_handle.set("delete", delete_fn)?;

                            // col:get(key) - read (non-transactional, just reads current state)
                            let storage_get = storage.clone();
                            let db_get = db_name.clone();
                            let coll_get = coll_name.clone();
                            let get_fn =
                                lua.create_function(move |lua, (_, key): (LuaValue, String)| {
                                    let full_coll_name = format!("{}:{}", db_get, coll_get);
                                    let collection = storage_get
                                        .get_collection(&full_coll_name)
                                        .map_err(mlua::Error::external)?;

                                    match collection.get(&key) {
                                        Ok(doc) => json_to_lua(lua, &doc.to_value()),
                                        Err(crate::error::DbError::DocumentNotFound(_)) => {
                                            Ok(LuaValue::Nil)
                                        }
                                        Err(e) => Err(mlua::Error::external(e)),
                                    }
                                })?;
                            coll_handle.set("get", get_fn)?;

                            Ok(LuaValue::Table(coll_handle))
                        })?;
                    tx_handle.set("collection", tx_collection_fn)?;

                    // Execute the callback with the transaction context
                    let result = callback
                        .call_async::<LuaValue>(LuaValue::Table(tx_handle))
                        .await;

                    match result {
                        Ok(value) => {
                            // Commit the transaction on success
                            storage
                                .commit_transaction(tx_id)
                                .map_err(mlua::Error::external)?;
                            Ok(value)
                        }
                        Err(e) => {
                            // Rollback on error
                            let _ = storage.rollback_transaction(tx_id);
                            Err(e)
                        }
                    }
                }
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create transaction function: {}", e))
            })?;

        db_handle.set("transaction", transaction_fn).map_err(|e| {
            DbError::InternalError(format!("Failed to set transaction function: {}", e))
        })?;

        // db:enqueue(queue, script, params, options)
        let storage_enqueue = self.storage.clone();
        let notifier_enqueue = self.queue_notifier.clone();
        let current_db_name = db_name.to_string();
        let enqueue_fn = lua
            .create_function(move |lua, args: mlua::MultiValue| {
                // Detect if called with colon (db:enqueue) or dot (db.enqueue)
                let (queue, script_path, params, options) = if args.len() >= 4
                    && matches!(args[0], LuaValue::Table(_))
                {
                    // Colon call: (self, queue, script, params, options)
                    let q = String::from_lua(args.get(1).cloned().unwrap_or(LuaValue::Nil), lua)?;
                    let s = String::from_lua(args.get(2).cloned().unwrap_or(LuaValue::Nil), lua)?;
                    let p = args.get(3).cloned().unwrap_or(LuaValue::Nil);
                    let o = args.get(4).cloned();
                    (q, s, p, o)
                } else {
                    // Dot call: (queue, script, params, options)
                    let q = String::from_lua(args.get(0).cloned().unwrap_or(LuaValue::Nil), lua)?;
                    let s = String::from_lua(args.get(1).cloned().unwrap_or(LuaValue::Nil), lua)?;
                    let p = args.get(2).cloned().unwrap_or(LuaValue::Nil);
                    let o = args.get(3).cloned();
                    (q, s, p, o)
                };

                let json_params = lua_to_json_value(lua, params)?;

                let mut priority = 0;
                let mut max_retries = 20;
                let mut run_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();

                if let Some(LuaValue::Table(t)) = options {
                    priority = t.get("priority").unwrap_or(0);
                    max_retries = t.get("max_retries").unwrap_or(20);
                    if let Ok(delay) = t.get::<u64>("run_at") {
                        run_at = delay;
                    }
                }

                let job_id = uuid::Uuid::new_v4().to_string();
                let job = crate::queue::Job {
                    id: job_id.clone(),
                    revision: None,
                    queue,
                    priority,
                    script_path,
                    params: json_params,
                    status: crate::queue::JobStatus::Pending,
                    retry_count: 0,
                    max_retries,
                    last_error: None,
                    cron_job_id: None,
                    run_at,
                    created_at: run_at,
                    started_at: None,
                    completed_at: None,
                };

                let db = storage_enqueue
                    .get_database(&current_db_name)
                    .map_err(mlua::Error::external)?;

                // Ensure _jobs collection exists
                if db.get_collection("_jobs").is_err() {
                    db.create_collection("_jobs".to_string(), None)
                        .map_err(mlua::Error::external)?;
                }

                let jobs_coll = db
                    .get_collection("_jobs")
                    .map_err(mlua::Error::external)?;

                let doc_val = serde_json::to_value(&job).unwrap();
                jobs_coll
                    .insert(doc_val)
                    .map_err(mlua::Error::external)?;

                // Notify worker
                if let Some(ref notifier) = notifier_enqueue {
                    let _ = notifier.send(());
                }

                Ok(job_id)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create enqueue function: {}", e))
            })?;

        db_handle.set("enqueue", enqueue_fn).map_err(|e| {
            DbError::InternalError(format!("Failed to set enqueue function: {}", e))
        })?;

        globals
            .set("db", db_handle)
            .map_err(|e| DbError::InternalError(format!("Failed to set db global: {}", e)))?;

        // Create 'request' table with context info
        let request = lua.create_table().map_err(|e| {
            DbError::InternalError(format!("Failed to create request table: {}", e))
        })?;

        request
            .set("method", context.method.clone())
            .map_err(|e| DbError::InternalError(format!("Failed to set method: {}", e)))?;
        request
            .set("path", context.path.clone())
            .map_err(|e| DbError::InternalError(format!("Failed to set path: {}", e)))?;

        // Query params
        let query = lua
            .create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create query table: {}", e)))?;
        for (k, v) in &context.query_params {
            query
                .set(k.clone(), v.clone())
                .map_err(|e| DbError::InternalError(format!("Failed to set query param: {}", e)))?;
        }
        request
            .set("query", query.clone())
            .map_err(|e| DbError::InternalError(format!("Failed to set query: {}", e)))?;
        request
            .set("query_params", query)
            .map_err(|e| DbError::InternalError(format!("Failed to set query_params: {}", e)))?;

        // URL params
        let params = lua
            .create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create params table: {}", e)))?;
        for (k, v) in &context.params {
            params
                .set(k.clone(), v.clone())
                .map_err(|e| DbError::InternalError(format!("Failed to set param: {}", e)))?;
        }
        request
            .set("params", params)
            .map_err(|e| DbError::InternalError(format!("Failed to set params: {}", e)))?;

        // Headers
        let headers = lua.create_table().map_err(|e| {
            DbError::InternalError(format!("Failed to create headers table: {}", e))
        })?;
        for (k, v) in &context.headers {
            headers
                .set(k.clone(), v.clone())
                .map_err(|e| DbError::InternalError(format!("Failed to set header: {}", e)))?;
        }
        request
            .set("headers", headers)
            .map_err(|e| DbError::InternalError(format!("Failed to set headers: {}", e)))?;

        // Body
        if let Some(body) = &context.body {
            let body_lua = json_to_lua(&lua, body)
                .map_err(|e| DbError::InternalError(format!("Failed to convert body: {}", e)))?;
            request
                .set("body", body_lua)
                .map_err(|e| DbError::InternalError(format!("Failed to set body: {}", e)))?;
        }

        request
            .set("is_websocket", context.is_websocket)
            .map_err(|e| DbError::InternalError(format!("Failed to set is_websocket: {}", e)))?;

        globals
            .set("request", request.clone())
            .map_err(|e| DbError::InternalError(format!("Failed to set request global: {}", e)))?;

        globals
            .set("context", request)
            .map_err(|e| DbError::InternalError(format!("Failed to set context global: {}", e)))?;

        // Create 'response' helper table
        let response = lua.create_table().map_err(|e| {
            DbError::InternalError(format!("Failed to create response table: {}", e))
        })?;

        // response.json(data) - helper to return JSON
        let json_fn = lua
            .create_function(|_, data: LuaValue| Ok(data))
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create json function: {}", e))
            })?;
        response
            .set("json", json_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set json: {}", e)))?;

        // response.html(content) - HTML response
        let html_fn = create_response_html_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create html function: {}", e))
        })?;
        response
            .set("html", html_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set html: {}", e)))?;

        // response.file(path) - file download
        let file_fn = create_response_file_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create file function: {}", e))
        })?;
        response
            .set("file", file_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set file: {}", e)))?;

        // response.stream(data) - streaming response
        let stream_fn = create_response_stream_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create stream function: {}", e))
        })?;
        response
            .set("stream", stream_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set stream: {}", e)))?;

        // response.cors(options) - CORS headers
        let cors_fn = create_response_cors_function(lua).map_err(|e| {
            DbError::InternalError(format!("Failed to create cors function: {}", e))
        })?;
        response
            .set("cors", cors_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set cors: {}", e)))?;

        globals
            .set("response", response)
            .map_err(|e| DbError::InternalError(format!("Failed to set response global: {}", e)))?;

        // Create 'crypto' namespace
        let crypto = lua
            .create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create crypto table: {}", e)))?;

        // md5(data)
        let md5_fn = lua
            .create_function(|_, data: mlua::String| {
                let digest = md5::compute(&data.as_bytes());
                Ok(format!("{:x}", digest))
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create md5 function: {}", e)))?;
        crypto
            .set("md5", md5_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set md5: {}", e)))?;

        // sha256(data)
        let sha256_fn = lua
            .create_function(|_, data: mlua::String| {
                let mut hasher = sha2::Sha256::new();
                hasher.update(&data.as_bytes());
                Ok(hex::encode(hasher.finalize()))
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create sha256 function: {}", e))
            })?;
        crypto
            .set("sha256", sha256_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set sha256: {}", e)))?;

        // sha512(data)
        let sha512_fn = lua
            .create_function(|_, data: mlua::String| {
                let mut hasher = sha2::Sha512::new();
                hasher.update(&data.as_bytes());
                Ok(hex::encode(hasher.finalize()))
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create sha512 function: {}", e))
            })?;
        crypto
            .set("sha512", sha512_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set sha512: {}", e)))?;

        // hmac_sha256(key, data)
        let hmac_sha256_fn = lua
            .create_function(|_, (key, data): (mlua::String, mlua::String)| {
                type HmacSha256 = hmac::Hmac<sha2::Sha256>;
                let mut mac = HmacSha256::new_from_slice(&key.as_bytes())
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
                mac.update(&data.as_bytes());
                Ok(hex::encode(mac.finalize().into_bytes()))
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create hmac_sha256 function: {}", e))
            })?;
        crypto
            .set("hmac_sha256", hmac_sha256_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set hmac_sha256: {}", e)))?;

        // hmac_sha512(key, data)
        let hmac_sha512_fn = lua
            .create_function(|_, (key, data): (mlua::String, mlua::String)| {
                type HmacSha512 = hmac::Hmac<sha2::Sha512>;
                let mut mac = HmacSha512::new_from_slice(&key.as_bytes())
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
                mac.update(&data.as_bytes());
                Ok(hex::encode(mac.finalize().into_bytes()))
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create hmac_sha512 function: {}", e))
            })?;
        crypto
            .set("hmac_sha512", hmac_sha512_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set hmac_sha512: {}", e)))?;

        // base64_encode(data)
        let base64_encode_fn = lua
            .create_function(|_, data: mlua::String| {
                Ok(base64::engine::general_purpose::STANDARD.encode(&data.as_bytes()))
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create base64_encode function: {}", e))
            })?;
        crypto
            .set("base64_encode", base64_encode_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set base64_encode: {}", e)))?;

        // base64_decode(data)
        let base64_decode_fn = lua
            .create_function(|lua, data: String| {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
                lua.create_string(&bytes)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create base64_decode function: {}", e))
            })?;
        crypto
            .set("base64_decode", base64_decode_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set base64_decode: {}", e)))?;

        // base32_encode(data)
        let base32_encode_fn = lua
            .create_function(|_, data: mlua::String| {
                let encoded = base32::encode(
                    base32::Alphabet::RFC4648 { padding: true },
                    &data.as_bytes(),
                );
                Ok(encoded)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create base32_encode function: {}", e))
            })?;
        crypto
            .set("base32_encode", base32_encode_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set base32_encode: {}", e)))?;

        // base32_decode(data)
        let base32_decode_fn = lua
            .create_function(|lua, data: String| {
                let bytes = base32::decode(base32::Alphabet::RFC4648 { padding: true }, &data)
                    .ok_or_else(|| mlua::Error::RuntimeError("Invalid base32".to_string()))?;
                lua.create_string(&bytes)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create base32_decode function: {}", e))
            })?;
        crypto
            .set("base32_decode", base32_decode_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set base32_decode: {}", e)))?;

        // hex_encode(data)
        let hex_encode_fn = lua
            .create_function(|_, data: String| Ok(hex::encode(data)))
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create hex_encode function: {}", e))
            })?;
        crypto
            .set("hex_encode", hex_encode_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set hex_encode: {}", e)))?;

        // hex_decode(data)
        let hex_decode_fn = lua
            .create_function(|lua, data: String| {
                let bytes =
                    hex::decode(data).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
                lua.create_string(&bytes)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create hex_decode function: {}", e))
            })?;
        crypto
            .set("hex_decode", hex_decode_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set hex_decode: {}", e)))?;

        // uuid()
        let uuid_fn = lua
            .create_function(|_, ()| Ok(uuid::Uuid::new_v4().to_string()))
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create uuid function: {}", e))
            })?;
        crypto
            .set("uuid", uuid_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set uuid: {}", e)))?;

        // uuid_v7()
        let uuid_v7_fn = lua
            .create_function(|_, ()| Ok(uuid::Uuid::now_v7().to_string()))
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create uuid_v7 function: {}", e))
            })?;
        crypto
            .set("uuid_v7", uuid_v7_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set uuid_v7: {}", e)))?;

        // random_bytes(len)
        let random_bytes_fn = lua
            .create_function(|lua, len: usize| {
                let mut bytes = vec![0u8; len];
                rand::thread_rng().fill_bytes(&mut bytes);
                lua.create_string(&bytes)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create random_bytes function: {}", e))
            })?;
        crypto
            .set("random_bytes", random_bytes_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set random_bytes: {}", e)))?;

        // curve25519(secret, public_or_basepoint)
        let curve25519_fn = lua
            .create_function(|lua, (secret, public): (mlua::String, mlua::String)| {
                let secret_bytes = secret.as_bytes();
                if secret_bytes.len() != 32 {
                    return Err(mlua::Error::RuntimeError(format!(
                        "Secret must be 32 bytes, got {}",
                        secret_bytes.len()
                    )));
                }
                let secret_slice: &[u8] = &secret_bytes;
                let secret_arr: [u8; 32] = secret_slice.try_into().unwrap();
                let secret_key = x25519_dalek::StaticSecret::from(secret_arr);

                let public_bytes = public.as_bytes();
                let public_slice: &[u8] = &public_bytes;
                if public_slice.len() == 32 {
                    // Shared secret calculation
                    let public_arr: [u8; 32] = public_slice.try_into().unwrap();
                    let public_key = x25519_dalek::PublicKey::from(public_arr);
                    let shared_secret = secret_key.diffie_hellman(&public_key);
                    lua.create_string(shared_secret.as_bytes())
                } else {
                    // Basepoint multiplication (Public Key generation)
                    let public_key = x25519_dalek::PublicKey::from(&secret_key);
                    lua.create_string(public_key.as_bytes())
                }
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create curve25519 function: {}", e))
            })?;
        crypto
            .set("curve25519", curve25519_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set curve25519: {}", e)))?;

        // hash_password(password)
        let hash_password_fn = lua
            .create_async_function(|_, password: String| async move {
                tokio::task::spawn_blocking(move || {
                    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
                    let argon2 = Argon2::default();
                    argon2
                        .hash_password(password.as_bytes(), &salt)
                        .map(|h| h.to_string())
                        .map_err(|e| mlua::Error::RuntimeError(e.to_string()))
                })
                .await
                .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create hash_password function: {}", e))
            })?;
        crypto
            .set("hash_password", hash_password_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set hash_password: {}", e)))?;

        // verify_password(hash, password)
        let verify_password_fn = lua
            .create_async_function(|_, (hash, password): (String, String)| async move {
                tokio::task::spawn_blocking(move || {
                    let parsed_hash = PasswordHash::new(&hash)
                        .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
                    Ok(Argon2::default()
                        .verify_password(password.as_bytes(), &parsed_hash)
                        .is_ok())
                })
                .await
                .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create verify_password function: {}", e))
            })?;
        crypto
            .set("verify_password", verify_password_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set verify_password: {}", e)))?;

        // jwt_encode(claims, secret)
        let jwt_encode_fn = lua
            .create_function(
                move |lua, (claims, secret): (LuaValue, String)| -> Result<String, mlua::Error> {
                    let json_claims = lua_to_json_value(lua, claims)?;
                    let token = encode(
                        &Header::default(),
                        &json_claims,
                        &EncodingKey::from_secret(secret.as_bytes()),
                    )
                    .map_err(|e| mlua::Error::RuntimeError(format!("JWT encode error: {}", e)))?;
                    Ok(token)
                },
            )
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create jwt_encode function: {}", e))
            })?;
        crypto
            .set("jwt_encode", jwt_encode_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set jwt_encode: {}", e)))?;

        // jwt_decode(token, secret)
        let jwt_decode_fn = lua
            .create_function(
                move |lua, (token, secret): (String, String)| -> Result<mlua::Value, mlua::Error> {
                    let token_data = decode::<serde_json::Value>(
                        &token,
                        &DecodingKey::from_secret(secret.as_bytes()),
                        &Validation::default(),
                    )
                    .map_err(|e| mlua::Error::RuntimeError(format!("JWT decode error: {}", e)))?;

                    json_to_lua(lua, &token_data.claims)
                },
            )
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create jwt_decode function: {}", e))
            })?;
        crypto
            .set("jwt_decode", jwt_decode_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set jwt_decode: {}", e)))?;

        globals
            .set("crypto", crypto)
            .map_err(|e| DbError::InternalError(format!("Failed to set crypto global: {}", e)))?;

        // Create 'time' namespace
        let time = lua
            .create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create time table: {}", e)))?;

        // time.now() -> float (seconds)
        let now_fn = lua
            .create_function(|_, ()| {
                let now = chrono::Utc::now();
                let ts =
                    now.timestamp() as f64 + now.timestamp_subsec_micros() as f64 / 1_000_000.0;
                Ok(ts)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create time.now function: {}", e))
            })?;
        time.set("now", now_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.now: {}", e)))?;

        // time.now_ms() -> int (milliseconds)
        let now_ms_fn = lua
            .create_function(|_, ()| Ok(chrono::Utc::now().timestamp_millis()))
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create time.now_ms function: {}", e))
            })?;
        time.set("now_ms", now_ms_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.now_ms: {}", e)))?;

        // time.iso() -> string
        let iso_fn = lua
            .create_function(|_, ()| Ok(chrono::Utc::now().to_rfc3339()))
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create time.iso function: {}", e))
            })?;
        time.set("iso", iso_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.iso: {}", e)))?;

        // time.sleep(ms) -> async
        let sleep_fn = lua
            .create_async_function(|_, ms: u64| async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(ms)).await;
                Ok(())
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create time.sleep function: {}", e))
            })?;
        time.set("sleep", sleep_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.sleep: {}", e)))?;

        // time.format(ts, format) -> string
        let format_fn = lua
            .create_function(|_, (ts, fmt): (f64, String)| {
                let secs = ts.trunc() as i64;
                let nsecs = (ts.fract() * 1_000_000_000.0) as u32;
                let dt = chrono::DateTime::from_timestamp(secs, nsecs)
                    .ok_or(mlua::Error::RuntimeError("Invalid timestamp".into()))?;
                Ok(dt.format(&fmt).to_string())
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create time.format function: {}", e))
            })?;
        time.set("format", format_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.format: {}", e)))?;

        // time.parse(iso) -> float
        let parse_fn = lua
            .create_function(|_, iso: String| {
                let dt = chrono::DateTime::parse_from_rfc3339(&iso)
                    .map_err(|e| mlua::Error::RuntimeError(format!("Parse error: {}", e)))?;
                let ts = dt.timestamp() as f64 + dt.timestamp_subsec_micros() as f64 / 1_000_000.0;
                Ok(ts)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create time.parse function: {}", e))
            })?;
        time.set("parse", parse_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.parse: {}", e)))?;

        // time.add(ts, value, unit) -> float
        let add_fn = lua
            .create_function(|_, (ts, val, unit): (f64, f64, String)| {
                let added_seconds = match unit.as_str() {
                    "ms" => val / 1000.0,
                    "s" => val,
                    "m" => val * 60.0,
                    "h" => val * 3600.0,
                    "d" => val * 86400.0,
                    _ => return Err(mlua::Error::RuntimeError(format!("Unknown unit: {}", unit))),
                };
                Ok(ts + added_seconds)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create time.add function: {}", e))
            })?;
        time.set("add", add_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.add: {}", e)))?;

        // time.subtract(ts, value, unit) -> float
        let sub_fn = lua
            .create_function(|_, (ts, val, unit): (f64, f64, String)| {
                let sub_seconds = match unit.as_str() {
                    "ms" => val / 1000.0,
                    "s" => val,
                    "m" => val * 60.0,
                    "h" => val * 3600.0,
                    "d" => val * 86400.0,
                    _ => return Err(mlua::Error::RuntimeError(format!("Unknown unit: {}", unit))),
                };
                Ok(ts - sub_seconds)
            })
            .map_err(|e| {
                DbError::InternalError(format!("Failed to create time.subtract function: {}", e))
            })?;
        time.set("subtract", sub_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set time.subtract: {}", e)))?;

        globals
            .set("time", time)
            .map_err(|e| DbError::InternalError(format!("Failed to set time global: {}", e)))?;

        Ok(())
    }

    /// Convert Lua value to JSON
    fn lua_to_json(&self, lua: &Lua, value: LuaValue) -> Result<JsonValue, DbError> {
        lua_to_json_value(lua, value)
            .map_err(|e| DbError::InternalError(format!("Failed to convert Lua to JSON: {}", e)))
    }
}

/// Result from script execution
#[derive(Debug)]
pub struct ScriptResult {
    pub status: u16,
    pub body: JsonValue,
    pub headers: std::collections::HashMap<String, String>,
}

/// Convert JSON value to Lua value
fn json_to_lua(lua: &Lua, json: &JsonValue) -> LuaResult<LuaValue> {
    match json {
        JsonValue::Null => Ok(LuaValue::Nil),
        JsonValue::Bool(b) => Ok(LuaValue::Boolean(*b)),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(LuaValue::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(LuaValue::Number(f))
            } else {
                Ok(LuaValue::Nil)
            }
        }
        JsonValue::String(s) => Ok(LuaValue::String(lua.create_string(s)?)),
        JsonValue::Array(arr) => {
            let table = lua.create_table()?;
            for (i, v) in arr.iter().enumerate() {
                table.set(i + 1, json_to_lua(lua, v)?)?;
            }
            Ok(LuaValue::Table(table))
        }
        JsonValue::Object(obj) => {
            let table = lua.create_table()?;
            for (k, v) in obj {
                table.set(k.clone(), json_to_lua(lua, v)?)?;
            }
            Ok(LuaValue::Table(table))
        }
    }
}

/// Convert Lua value to JSON value
fn lua_value_to_json(value: &LuaValue) -> LuaResult<JsonValue> {
    match value {
        LuaValue::Nil => Ok(JsonValue::Null),
        LuaValue::Boolean(b) => Ok(JsonValue::Bool(*b)),
        LuaValue::Integer(i) => Ok(JsonValue::Number((*i).into())),
        LuaValue::Number(n) => Ok(serde_json::Number::from_f64(*n)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null)),
        LuaValue::String(s) => Ok(JsonValue::String(s.to_str()?.to_string())),
        LuaValue::Table(t) => {
            // Check if it's an array (sequential integer keys starting from 1)
            let mut is_array = true;
            let mut max_key = 0i64;
            for pair in t.clone().pairs::<LuaValue, LuaValue>() {
                let (k, _) = pair?;
                match k {
                    LuaValue::Integer(i) if i > 0 => {
                        max_key = max_key.max(i);
                    }
                    _ => {
                        is_array = false;
                        break;
                    }
                }
            }

            if is_array && max_key > 0 {
                // It's an array
                let mut arr = Vec::new();
                for i in 1..=max_key {
                    let val: LuaValue = t.get(i)?;
                    arr.push(lua_value_to_json(&val)?);
                }
                Ok(JsonValue::Array(arr))
            } else {
                // It's an object
                let mut map = serde_json::Map::new();
                for pair in t.clone().pairs::<LuaValue, LuaValue>() {
                    let (k, v) = pair?;
                    let key_str = match k {
                        LuaValue::String(s) => s.to_str()?.to_string(),
                        LuaValue::Integer(i) => i.to_string(),
                        LuaValue::Number(n) => n.to_string(),
                        _ => continue,
                    };
                    map.insert(key_str, lua_value_to_json(&v)?);
                }
                Ok(JsonValue::Object(map))
            }
        }
        _ => Ok(JsonValue::Null),
    }
}

/// Check if a document matches a filter
/// Supports simple equality matching on fields
fn matches_filter(doc: &JsonValue, filter: &JsonValue) -> bool {
    match filter {
        JsonValue::Object(filter_obj) => {
            for (key, filter_value) in filter_obj {
                match doc.get(key) {
                    Some(doc_value) => {
                        // Simple equality check
                        if doc_value != filter_value {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
            true
        }
        _ => false,
    }
}

/// Convert Lua value to JSON value
fn lua_to_json_value(lua: &Lua, value: LuaValue) -> LuaResult<JsonValue> {
    match value {
        LuaValue::Nil => Ok(JsonValue::Null),
        LuaValue::Boolean(b) => Ok(JsonValue::Bool(b)),
        LuaValue::Integer(i) => Ok(JsonValue::Number(i.into())),
        LuaValue::Number(n) => Ok(serde_json::Number::from_f64(n)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null)),
        LuaValue::String(s) => Ok(JsonValue::String(s.to_str()?.to_string())),
        LuaValue::Table(t) => {
            // Check if it's an array (sequential integer keys starting from 1)
            let mut is_array = true;
            let mut max_index = 0;

            for pair in t.clone().pairs::<LuaValue, LuaValue>() {
                let (k, _) = pair?;
                if let LuaValue::Integer(i) = k {
                    if i > 0 {
                        max_index = max_index.max(i);
                    } else {
                        is_array = false;
                        break;
                    }
                } else {
                    is_array = false;
                    break;
                }
            }

            if is_array && max_index > 0 {
                let mut arr = Vec::with_capacity(max_index as usize);
                for i in 1..=max_index {
                    let v: LuaValue = t.get(i)?;
                    arr.push(lua_to_json_value(lua, v)?);
                }
                Ok(JsonValue::Array(arr))
            } else {
                let mut obj = serde_json::Map::new();
                for pair in t.pairs::<String, LuaValue>() {
                    let (k, v) = pair?;
                    obj.insert(k, lua_to_json_value(lua, v)?);
                }
                Ok(JsonValue::Object(obj))
            }
        }
        _ => Ok(JsonValue::Null), // Functions, userdata, etc. become null
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_lua_roundtrip() {
        let lua = Lua::new();

        let json = serde_json::json!({
            "name": "test",
            "count": 42,
            "active": true,
            "tags": ["a", "b", "c"]
        });

        let lua_val = json_to_lua(&lua, &json).unwrap();
        let back = lua_to_json_value(&lua, lua_val).unwrap();

        assert_eq!(json, back);
    }
}
