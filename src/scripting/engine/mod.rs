use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use mlua::{Lua, Value as LuaValue};
use serde_json::Value as JsonValue;
use tokio::sync::broadcast;

use crate::error::DbError;
use crate::scripting::channel_manager::ChannelManager;
use crate::storage::StorageEngine;
use crate::stream::StreamManager;

use super::conversion::lua_to_json_value;
use super::types::{Script, ScriptContext, ScriptResult, ScriptStats};

pub mod cache;
pub mod globals;
pub mod pool;
pub mod repl;
pub mod script_index;
pub mod websocket;

pub use cache::ScriptCache;
pub use pool::LuaPool;
pub use script_index::ScriptIndex;

/// Lua scripting engine
pub struct ScriptEngine {
    pub(crate) storage: Arc<StorageEngine>,
    pub(crate) queue_notifier: Option<broadcast::Sender<()>>,
    pub(crate) stream_manager: Option<Arc<StreamManager>>,
    pub(crate) channel_manager: Option<Arc<ChannelManager>>,
    pub(crate) stats: Arc<ScriptStats>,
    /// Optional Lua VM pool for efficient state reuse
    pub(crate) lua_pool: Option<Arc<LuaPool>>,
    /// Optional bytecode cache for avoiding recompilation
    pub(crate) script_cache: Option<Arc<ScriptCache>>,
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
            lua_pool: None,
            script_cache: None,
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

    /// Configure the engine to use a Lua VM pool for efficient state reuse.
    ///
    /// This dramatically reduces per-request overhead by reusing pre-initialized
    /// Lua states instead of creating new ones for each request.
    pub fn with_lua_pool(mut self, pool: Arc<LuaPool>) -> Self {
        self.lua_pool = Some(pool);
        self
    }

    /// Configure the engine to use a bytecode cache.
    ///
    /// This avoids recompiling scripts on every request by caching
    /// the compiled bytecode.
    pub fn with_script_cache(mut self, cache: Arc<ScriptCache>) -> Self {
        self.script_cache = Some(cache);
        self
    }

    /// Execute a Lua script with the given context
    pub async fn execute(
        &self,
        script: &Script,
        db_name: &str,
        context: &ScriptContext,
    ) -> Result<ScriptResult, DbError> {
        // Use Relaxed ordering for stats - exact counts not critical for performance
        self.stats.active_scripts.fetch_add(1, Ordering::Relaxed);
        self.stats
            .total_scripts_executed
            .fetch_add(1, Ordering::Relaxed);

        // Ensure active counter is decremented even on panic or early return
        // Use a reference to avoid Arc clone overhead
        struct ActiveScriptGuard<'a>(&'a ScriptStats);
        impl Drop for ActiveScriptGuard<'_> {
            fn drop(&mut self) {
                self.0.active_scripts.fetch_sub(1, Ordering::Relaxed);
            }
        }
        let _guard = ActiveScriptGuard(&self.stats);

        // Use pooled Lua state if available, otherwise create a new one
        if let Some(ref pool) = self.lua_pool {
            self.execute_with_pool(pool, script, db_name, context).await
        } else {
            self.execute_without_pool(script, db_name, context).await
        }
    }

    /// Execute using a pooled Lua state (fast path)
    ///
    /// This method uses a two-tier globals system for maximum performance:
    /// - Static globals (crypto, time, json, etc.) are initialized once per pool state
    /// - Per-request globals (db, request, context, etc.) are set up on each request
    async fn execute_with_pool(
        &self,
        pool: &Arc<LuaPool>,
        script: &Script,
        db_name: &str,
        context: &ScriptContext,
    ) -> Result<ScriptResult, DbError> {
        let pool_guard = pool.acquire();

        // Check if this is a "pure" script that doesn't need globals setup
        // Pure scripts don't reference db, request, response, solidb, context
        let needs_globals = !pool.skip_reset()
            || script.code.contains("db.")
            || script.code.contains("db:")
            || script.code.contains("request")
            || script.code.contains("response")
            || script.code.contains("solidb")
            || script.code.contains("context");

        // SINGLE lock acquisition for entire operation
        let result = pool_guard.with_lua(|lua| {
            // 1. Set up the Lua environment (skip for pure scripts in fast mode)
            if needs_globals {
                // Check if static globals are already initialized (two-tier optimization)
                let has_static_globals = lua
                    .globals()
                    .get::<bool>("__solidb_static_initialized")
                    .unwrap_or(false);

                if has_static_globals {
                    // Fast path: only set up per-request globals
                    globals::setup_request_globals(
                        self,
                        lua,
                        db_name,
                        context,
                        Some((&script.key, &script.name)),
                    )?;
                } else {
                    // Fallback: set up all globals (for states without static initialization)
                    self.setup_lua_globals(
                        lua,
                        db_name,
                        context,
                        Some((&script.key, &script.name)),
                    )?;
                }
            }

            // 2. Get or compile bytecode
            let bytecode = if let Some(ref cache) = self.script_cache {
                cache
                    .get_or_compile(&script.key, &script.code, |code| {
                        let chunk = lua.load(code);
                        let func = chunk.into_function()?;
                        Ok(func.dump(false))
                    })
                    .map_err(|e| {
                        DbError::InternalError(format!("Bytecode compilation error: {}", e))
                    })?
            } else {
                // No cache - compile directly
                let chunk = lua.load(&script.code);
                let func = chunk.into_function().map_err(|e| {
                    DbError::InternalError(format!("Script compilation error: {}", e))
                })?;
                func.dump(false)
            };

            // 3. Execute the bytecode
            let chunk = lua.load(&bytecode[..]);
            let lua_result = chunk
                .eval::<LuaValue>()
                .map_err(|e| DbError::InternalError(format!("Lua error: {}", e)))?;

            // 4. Convert result to JSON
            self.lua_to_json(lua, lua_result)
        })?;

        Ok(ScriptResult {
            status: 200,
            body: result,
            headers: HashMap::new(),
        })
    }

    /// Execute without pooling (original behavior, used as fallback)
    async fn execute_without_pool(
        &self,
        script: &Script,
        db_name: &str,
        context: &ScriptContext,
    ) -> Result<ScriptResult, DbError> {
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
