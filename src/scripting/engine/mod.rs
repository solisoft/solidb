use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use mlua::{Value as LuaValue, Lua};
use serde_json::Value as JsonValue;
use tokio::sync::broadcast;

use crate::error::DbError;
use crate::storage::StorageEngine;
use crate::stream::StreamManager;
use crate::scripting::channel_manager::ChannelManager;

use super::conversion::lua_to_json_value;
use super::types::{Script, ScriptContext, ScriptResult, ScriptStats};

pub mod websocket;
pub mod repl;
pub mod globals;

/// Lua scripting engine
pub struct ScriptEngine {
    pub(crate) storage: Arc<StorageEngine>,
    pub(crate) queue_notifier: Option<broadcast::Sender<()>>,
    pub(crate) stream_manager: Option<Arc<StreamManager>>,
    pub(crate) channel_manager: Option<Arc<ChannelManager>>,
    pub(crate) stats: Arc<ScriptStats>,
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
        websocket::execute_ws(self, script, db_name, context, ws).await
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
        repl::execute_repl(self, code, db_name, variables, history, output_capture).await
    }

    // Helper exposed for submodules
    pub(crate) fn setup_lua_globals(
        &self,
        lua: &Lua,
        db_name: &str,
        context: &ScriptContext,
        script_info: Option<(&str, &str)>,
    ) -> Result<(), DbError> {
        globals::setup_lua_globals(self, lua, db_name, context, script_info)
    }

    /// Convert Lua value to JSON
    pub(crate) fn lua_to_json(&self, lua: &Lua, value: LuaValue) -> Result<JsonValue, DbError> {
        lua_to_json_value(lua, value)
            .map_err(|e| DbError::InternalError(format!("Failed to convert Lua to JSON: {}", e)))
    }
}
