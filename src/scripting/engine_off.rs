//! `scripting::engine` in a binary built without the `lua` feature.
//!
//! The same names as the Lua engine, so that the server, the queue and the
//! handlers compile unchanged: scripts and services are still stored, listed
//! and routed (`ScriptIndex` is the real one), and every attempt to run one
//! is refused with `lua_disabled_error`, as `--no-lua` refuses it in a binary
//! that has Lua. `LuaPool` cannot be built, so `AppState::lua_pool` stays
//! `None`.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value as JsonValue;
use tokio::sync::broadcast;

use crate::error::DbError;
use crate::scripting::channel_manager::ChannelManager;
use crate::scripting::types::{Script, ScriptContext, ScriptResult, ScriptStats};
use crate::storage::StorageEngine;
use crate::stream::StreamManager;

#[path = "engine/script_index.rs"]
pub mod script_index;

pub use script_index::ScriptIndex;

/// Always false: there is no Lua runtime in this binary.
pub fn lua_runtime_enabled() -> bool {
    false
}

pub fn lua_disabled_error() -> DbError {
    DbError::OperationNotSupported(
        "Lua is not built into this binary (build with `--features lua`). Custom scripts, \
         services, and the Lua REPL are not available."
            .to_string(),
    )
}

/// The Lua VM pool. Has no constructor here: nothing can hold one.
pub enum LuaPool {}

/// The bytecode cache: holds nothing, there is nothing to compile.
#[derive(Default)]
pub struct ScriptCache;

impl ScriptCache {
    pub fn new(_max_size: usize) -> Self {
        Self
    }

    pub fn with_default_size() -> Self {
        Self
    }

    pub fn invalidate(&self, _script_key: &str) {}

    pub fn clear(&self) {}
}

/// The script engine: built and configured like the Lua one, refuses to run.
#[derive(Clone)]
pub struct ScriptEngine {
    _stats: Arc<ScriptStats>,
}

impl ScriptEngine {
    pub fn new(_storage: Arc<StorageEngine>, stats: Arc<ScriptStats>) -> Self {
        Self { _stats: stats }
    }

    pub fn with_queue_notifier(self, _notifier: broadcast::Sender<()>) -> Self {
        self
    }

    pub fn with_stream_manager(self, _manager: Arc<StreamManager>) -> Self {
        self
    }

    pub fn with_channel_manager(self, _manager: Arc<ChannelManager>) -> Self {
        self
    }

    pub fn with_lua_pool(self, pool: Arc<LuaPool>) -> Self {
        match *pool {}
    }

    pub fn with_script_cache(self, _cache: Arc<ScriptCache>) -> Self {
        self
    }

    pub async fn execute(
        &self,
        _script: &Script,
        _db_name: &str,
        _context: &ScriptContext,
    ) -> Result<ScriptResult, DbError> {
        Err(lua_disabled_error())
    }

    pub async fn execute_ws(
        &self,
        _script: &Script,
        _db_name: &str,
        _context: &ScriptContext,
        _ws: axum::extract::ws::WebSocket,
    ) -> Result<(), DbError> {
        Err(lua_disabled_error())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn execute_repl(
        &self,
        _code: &str,
        _db_name: &str,
        _user: crate::scripting::auth::ScriptUser,
        _variables: &HashMap<String, JsonValue>,
        _history: &[String],
        _output_capture: &mut Vec<String>,
        _timeout_ms: u64,
    ) -> Result<(JsonValue, HashMap<String, JsonValue>), DbError> {
        Err(lua_disabled_error())
    }
}
