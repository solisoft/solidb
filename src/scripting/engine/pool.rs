//! Lua VM Pool for efficient state reuse
//!
//! This module provides a pool of pre-initialized Lua VMs that can be
//! borrowed and returned for script execution, avoiding the overhead
//! of creating new VMs for every request.
//!
//! ## Two-Tier Globals Optimization
//!
//! The pool implements a two-tier globals system for maximum performance:
//!
//! **Tier 1 - Static Globals** (initialized once per pool state):
//! - `crypto.*` - md5, sha256, jwt, password hashing, etc.
//! - `time.*` - now, millis, date, parse, iso, diff, add, subtract, format
//! - `json.*` - encode, decode
//! - `string.*` extensions - regex, slugify, truncate, split, trim, pad_*
//! - `table.*` extensions - sorted, keys, values, merge, filter, map, find
//! - `response.*` - json, html, file, stream, cors
//! - `solidb.*` static functions - validate, sanitize, typeof, redirect, cache, error handling, dev tools
//!
//! **Tier 2 - Per-Request Globals** (set each request, but much faster):
//! - `request` / `context` - from ScriptContext
//! - `db` - needs db_name, storage
//! - `solidb.auth` - needs context.user
//! - `solidb.log` - needs db_name, script_info
//! - `solidb.env` - loaded from _env collection (cached)
//! - `solidb.file_*`, `solidb.upload`, `solidb.image_process` - need db_name
//! - `solidb.ai` - needs db_name
//! - `solidb.streams` - needs stream_manager

use mlua::{Lua, Value as LuaValue};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::error::DbError;
use crate::scripting::dev_tools::*;
use crate::scripting::error_handling::*;
use crate::scripting::http_helpers::*;
use crate::scripting::lua_globals;
use crate::scripting::validation::*;

/// A pool of pre-initialized Lua VMs for efficient reuse.
///
/// Creating a new Lua VM is expensive (~40% of request time for simple scripts).
/// This pool maintains a set of pre-sanitized Lua states that can be borrowed
/// and returned, dramatically reducing per-request overhead.
pub struct LuaPool {
    /// The pool of available Lua states
    states: Vec<Arc<PooledState>>,
    /// Round-robin counter for state selection
    next_index: AtomicUsize,
    /// Pool size
    size: usize,
    /// Skip global reset between requests (for pure/stateless scripts)
    skip_reset: bool,
}

/// Wrapper around a Lua state with usage tracking
struct PooledState {
    /// The Lua state (protected by Mutex for actual access)
    lua: Mutex<Lua>,
    /// Number of times this state has been used
    use_count: AtomicUsize,
    /// Whether the state is currently in use (lock-free acquisition)
    in_use: AtomicBool,
    /// Whether the state needs reset before next use (lazy reset)
    needs_reset: AtomicBool,
}

impl LuaPool {
    /// Create a new pool with the specified number of Lua states.
    ///
    /// Each state is pre-initialized and sanitized (unsafe globals removed).
    /// The pool size should typically match the number of worker threads.
    pub fn new(size: usize) -> Self {
        Self::new_with_options(size, false)
    }

    /// Create a new pool with options.
    ///
    /// If `skip_reset` is true, globals are NOT reset between requests.
    /// This is safe for stateless/pure scripts and provides maximum performance.
    ///
    /// Each pool state is pre-initialized with:
    /// 1. Sanitized globals (unsafe stdlib removed)
    /// 2. Static globals (crypto, time, json, string/table extensions, etc.)
    pub fn new_with_options(size: usize, skip_reset: bool) -> Self {
        let states = (0..size)
            .map(|_| {
                let lua = Lua::new();
                Self::sanitize_globals(&lua);
                // Initialize static globals ONCE per pool state
                Self::setup_static_globals(&lua);
                Arc::new(PooledState {
                    lua: Mutex::new(lua),
                    use_count: AtomicUsize::new(0),
                    in_use: AtomicBool::new(false),
                    needs_reset: AtomicBool::new(false),
                })
            })
            .collect();

        Self {
            states,
            next_index: AtomicUsize::new(0),
            size,
            skip_reset,
        }
    }

    /// Create a high-performance pool optimized for stateless scripts.
    ///
    /// This pool skips global reset between requests, providing maximum throughput
    /// for scripts that don't rely on clean global state.
    pub fn new_fast(size: usize) -> Self {
        Self::new_with_options(size, true)
    }

    /// Create a pool sized to the available parallelism.
    pub fn with_default_size() -> Self {
        let size = std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(4)
            .max(4); // Minimum 4 states

        // Check environment variable for fast mode
        let fast_mode = std::env::var("SOLIDB_LUA_FAST_MODE")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false);

        Self::new_with_options(size, fast_mode)
    }

    /// Create a high-performance pool with auto-sized parallelism.
    /// Skips global reset between requests for maximum throughput.
    pub fn with_default_size_fast() -> Self {
        let size = std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(4)
            .max(4);
        Self::new_fast(size)
    }

    /// Returns whether this pool skips reset between requests.
    pub fn skip_reset(&self) -> bool {
        self.skip_reset
    }

    /// Acquire a Lua state from the pool.
    ///
    /// This method uses lock-free round-robin selection to distribute load.
    /// Uses atomic compare_exchange for contention-free acquisition.
    pub fn acquire(&self) -> PoolGuard {
        let start = self.next_index.fetch_add(1, Ordering::Relaxed) % self.size;

        // Lock-free try-acquire loop
        for i in 0..self.size {
            let idx = (start + i) % self.size;
            let state = &self.states[idx];

            // Atomic compare-and-swap - NO MUTEX for checking availability
            if state
                .in_use
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                state.use_count.fetch_add(1, Ordering::Relaxed);
                return PoolGuard {
                    state: state.clone(),
                    index: idx,
                    skip_reset: self.skip_reset,
                };
            }
        }

        // All states busy - spin-wait on the first one (fallback)
        let state = &self.states[start];
        loop {
            if state
                .in_use
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                state.use_count.fetch_add(1, Ordering::Relaxed);
                return PoolGuard {
                    state: state.clone(),
                    index: start,
                    skip_reset: self.skip_reset,
                };
            }
            std::hint::spin_loop();
        }
    }

    /// Sanitize a Lua state by removing dangerous globals.
    ///
    /// This removes:
    /// - os: System operations
    /// - io: File I/O
    /// - debug: Debug interface
    /// - package: Module system
    /// - dofile, load, loadfile, require: Code loading
    fn sanitize_globals(lua: &Lua) {
        let globals = lua.globals();

        // Remove unsafe globals
        let unsafe_globals = [
            "os", "io", "debug", "package", "dofile", "load", "loadfile", "require",
        ];

        for name in &unsafe_globals {
            let _ = globals.set(*name, LuaValue::Nil);
        }
    }

    /// Initialize static globals that don't depend on request context.
    ///
    /// This is called ONCE per pool state creation, not per request.
    /// Static globals include: crypto, time, json, string extensions,
    /// table extensions, response helpers, validation functions, and dev tools.
    ///
    /// Returns true if setup succeeded, false otherwise.
    pub fn setup_static_globals(lua: &Lua) -> bool {
        // Each setup function logs its own errors
        if let Err(e) = Self::setup_static_globals_inner(lua) {
            tracing::warn!("Failed to setup static globals: {}", e);
            return false;
        }
        true
    }

    fn setup_static_globals_inner(lua: &Lua) -> Result<(), DbError> {
        let globals = lua.globals();

        // 1. Setup crypto namespace (md5, sha256, jwt, password hashing, etc.)
        lua_globals::setup_crypto_globals(lua)?;

        // 2. Setup time globals (time.now, time.date, time.parse, etc.)
        lua_globals::setup_time_globals(lua)?;

        // 3. Setup extended time functions (time.now_ms, time.sleep, time.format, etc.)
        lua_globals::setup_time_ext_globals(lua)?;

        // 4. Setup JSON globals (json.encode, json.decode)
        lua_globals::setup_json_globals_static(lua)?;

        // 5. Setup string library extensions (regex, slugify, truncate, split, trim, pad_*)
        lua_globals::setup_string_extensions(lua)?;

        // 6. Setup table library extensions (deep_merge, keys, values, contains, filter, map)
        lua_globals::setup_table_lib_extensions(lua)?;

        // 7. Setup pure Lua table extensions (sorted, merge, find, reverse, slice, len)
        lua_globals::setup_table_extensions(lua)?;

        // 8. Create solidb namespace with static functions
        let solidb = lua
            .create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create solidb table: {}", e)))?;

        // solidb.now() -> Unix timestamp (static, doesn't need db)
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

        // solidb.fetch - HTTP client (static)
        let fetch_fn = lua_globals::create_fetch_function(lua)?;
        solidb
            .set("fetch", fetch_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set fetch: {}", e)))?;

        // Validation functions (static)
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

        // HTTP helpers (static)
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

        // Error handling functions (static)
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

        // Development tools (static)
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

        // Add json_encode and json_decode to solidb namespace for compatibility
        let json_table: mlua::Table = globals
            .get("json")
            .map_err(|e| DbError::InternalError(format!("Failed to get json table: {}", e)))?;
        let json_encode: mlua::Function = json_table
            .get("encode")
            .map_err(|e| DbError::InternalError(format!("Failed to get json.encode: {}", e)))?;
        let json_decode: mlua::Function = json_table
            .get("decode")
            .map_err(|e| DbError::InternalError(format!("Failed to get json.decode: {}", e)))?;
        solidb
            .set("json_encode", json_encode)
            .map_err(|e| DbError::InternalError(format!("Failed to set json_encode: {}", e)))?;
        solidb
            .set("json_decode", json_decode)
            .map_err(|e| DbError::InternalError(format!("Failed to set json_decode: {}", e)))?;

        // 9. Create response table with static helpers
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

        // Set globals
        globals
            .set("solidb", solidb)
            .map_err(|e| DbError::InternalError(format!("Failed to set solidb global: {}", e)))?;
        globals
            .set("response", response)
            .map_err(|e| DbError::InternalError(format!("Failed to set response global: {}", e)))?;

        // Mark state as having static globals initialized
        globals
            .set("__solidb_static_initialized", true)
            .map_err(|e| {
                DbError::InternalError(format!("Failed to set static initialized marker: {}", e))
            })?;

        Ok(())
    }

    /// Reset a Lua state for reuse.
    ///
    /// This clears user-defined globals while preserving:
    /// - Standard Lua globals (pairs, ipairs, math, string, table, etc.)
    /// - Static globals initialized by the pool (crypto, time, json, solidb, response)
    fn reset_state(lua: &Lua) {
        let globals = lua.globals();

        // List of globals that should be preserved
        let preserved = [
            // Standard Lua globals
            "_G",
            "_VERSION",
            "assert",
            "collectgarbage",
            "error",
            "getmetatable",
            "ipairs",
            "next",
            "pairs",
            "pcall",
            "print",
            "rawequal",
            "rawget",
            "rawlen",
            "rawset",
            "select",
            "setmetatable",
            "tonumber",
            "tostring",
            "type",
            "xpcall",
            // Standard libraries we keep
            "coroutine",
            "math",
            "string",
            "table",
            "utf8",
            // Static globals initialized by pool (Tier 1)
            "crypto",
            "time",
            "json",
            "solidb",
            "response",
            // Marker for static initialization
            "__solidb_static_initialized",
        ];

        // Collect keys to remove (these are per-request globals like db, request, context)
        let mut to_remove = Vec::new();

        let pairs = globals.pairs::<String, LuaValue>();
        for pair in pairs.flatten() {
            let (key, _) = pair;
            if !preserved.contains(&key.as_str()) {
                to_remove.push(key);
            }
        }

        // Remove non-preserved globals (db, request, context, etc.)
        for key in to_remove {
            let _ = globals.set(key, LuaValue::Nil);
        }

        // Reset per-request fields in solidb table (auth, log, env, file functions, ai, streams, stats)
        // These will be re-set by setup_request_globals
        if let Ok(solidb) = globals.get::<mlua::Table>("solidb") {
            let _ = solidb.set("auth", LuaValue::Nil);
            let _ = solidb.set("log", LuaValue::Nil);
            let _ = solidb.set("env", LuaValue::Nil);
            let _ = solidb.set("upload", LuaValue::Nil);
            let _ = solidb.set("file_info", LuaValue::Nil);
            let _ = solidb.set("file_read", LuaValue::Nil);
            let _ = solidb.set("file_delete", LuaValue::Nil);
            let _ = solidb.set("file_list", LuaValue::Nil);
            let _ = solidb.set("image_process", LuaValue::Nil);
            let _ = solidb.set("ai", LuaValue::Nil);
            let _ = solidb.set("streams", LuaValue::Nil);
            let _ = solidb.set("stats", LuaValue::Nil);
        }
    }

    /// Get pool statistics (lock-free)
    pub fn stats(&self) -> PoolStats {
        let mut in_use = 0;
        let mut total_uses = 0;

        for state in &self.states {
            if state.in_use.load(Ordering::Relaxed) {
                in_use += 1;
            }
            total_uses += state.use_count.load(Ordering::Relaxed);
        }

        PoolStats {
            size: self.size,
            in_use,
            total_uses,
        }
    }
}

/// RAII guard for a borrowed Lua state.
///
/// When dropped, the state is marked for lazy reset and returned to the pool.
pub struct PoolGuard {
    state: Arc<PooledState>,
    #[allow(dead_code)]
    index: usize,
    /// Whether to skip reset (for fast/stateless mode)
    skip_reset: bool,
}

impl PoolGuard {
    /// Get a reference to the Lua state.
    ///
    /// Note: The caller should NOT hold this reference across await points.
    /// Use `with_lua` for operations that need the Lua state.
    /// Performs lazy reset if the state was previously used (unless skip_reset is true).
    pub fn with_lua<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Lua) -> R,
    {
        let guard = self.state.lua.lock();

        // Lazy reset: only reset if needed AND not in skip_reset mode
        if !self.skip_reset && self.state.needs_reset.swap(false, Ordering::Acquire) {
            LuaPool::reset_state(&guard);
        }

        f(&guard)
    }

    /// Get mutable access to the Lua state for setup operations.
    /// Performs lazy reset if the state was previously used (unless skip_reset is true).
    pub fn with_lua_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Lua) -> R,
    {
        let guard = self.state.lua.lock();

        // Lazy reset: only reset if needed AND not in skip_reset mode
        if !self.skip_reset && self.state.needs_reset.swap(false, Ordering::Acquire) {
            LuaPool::reset_state(&guard);
        }

        f(&guard)
    }
}

impl Drop for PoolGuard {
    fn drop(&mut self) {
        // Mark for lazy reset on next use (unless skip_reset mode)
        if !self.skip_reset {
            self.state.needs_reset.store(true, Ordering::Release);
        }
        // Release the state back to pool (lock-free)
        self.state.in_use.store(false, Ordering::Release);
    }
}

/// Statistics about the Lua pool
#[derive(Debug, Clone)]
pub struct PoolStats {
    /// Total number of states in the pool
    pub size: usize,
    /// Number of states currently in use
    pub in_use: usize,
    /// Total number of times states have been borrowed
    pub total_uses: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_creation() {
        let pool = LuaPool::new(4);
        assert_eq!(pool.size, 4);

        let stats = pool.stats();
        assert_eq!(stats.size, 4);
        assert_eq!(stats.in_use, 0);
    }

    #[test]
    fn test_pool_acquire_release() {
        let pool = LuaPool::new(2);

        {
            let guard1 = pool.acquire();
            let stats = pool.stats();
            assert_eq!(stats.in_use, 1);

            // Execute some Lua
            guard1.with_lua(|lua| {
                let result: i32 = lua.load("return 1 + 1").eval().unwrap();
                assert_eq!(result, 2);
            });
        }

        // After drop, state should be released
        let stats = pool.stats();
        assert_eq!(stats.in_use, 0);
        assert_eq!(stats.total_uses, 1);
    }

    #[test]
    fn test_globals_sanitized() {
        let pool = LuaPool::new(1);
        let guard = pool.acquire();

        guard.with_lua(|lua| {
            // os should be nil
            let result: LuaValue = lua.load("return os").eval().unwrap();
            assert!(matches!(result, LuaValue::Nil));

            // io should be nil
            let result: LuaValue = lua.load("return io").eval().unwrap();
            assert!(matches!(result, LuaValue::Nil));

            // But math should work
            let result: f64 = lua.load("return math.sqrt(4)").eval().unwrap();
            assert_eq!(result, 2.0);
        });
    }

    #[test]
    fn test_state_reset() {
        let pool = LuaPool::new(1);

        // First use: set a global
        {
            let guard = pool.acquire();
            guard.with_lua(|lua| {
                lua.load("my_global = 42").exec().unwrap();
                let result: i32 = lua.load("return my_global").eval().unwrap();
                assert_eq!(result, 42);
            });
        }

        // Second use: global should be cleared
        {
            let guard = pool.acquire();
            guard.with_lua(|lua| {
                let result: LuaValue = lua.load("return my_global").eval().unwrap();
                assert!(matches!(result, LuaValue::Nil));
            });
        }
    }

    #[test]
    fn test_concurrent_acquire() {
        use std::thread;

        let pool = Arc::new(LuaPool::new(4));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let p = pool.clone();
                thread::spawn(move || {
                    let guard = p.acquire();
                    guard.with_lua(|lua| {
                        // Each thread executes a simple computation
                        let result: i32 = lua
                            .load(&format!("return {} + 1", i))
                            .eval()
                            .expect("Lua eval failed");
                        assert_eq!(result, i + 1);
                    });
                })
            })
            .collect();

        for h in handles {
            h.join().expect("Thread panicked");
        }

        // All states should be released
        let stats = pool.stats();
        assert_eq!(stats.in_use, 0);
        assert_eq!(stats.total_uses, 8);
    }

    #[test]
    fn test_all_unsafe_globals_removed() {
        let pool = LuaPool::new(1);
        let guard = pool.acquire();

        guard.with_lua(|lua| {
            // All these globals should be nil for security
            let unsafe_globals = [
                "os", "io", "debug", "package", "dofile", "load", "loadfile", "require",
            ];

            for name in &unsafe_globals {
                let result: LuaValue = lua
                    .load(&format!("return {}", name))
                    .eval()
                    .expect("Eval failed");
                assert!(
                    matches!(result, LuaValue::Nil),
                    "{} should be nil but was {:?}",
                    name,
                    result
                );
            }
        });
    }

    #[test]
    fn test_pool_size_one_state_reuse() {
        let pool = LuaPool::new(1);

        // First use: set a global
        {
            let guard = pool.acquire();
            guard.with_lua(|lua| {
                lua.load("x = 1").exec().unwrap();
            });
        }

        // Second use: global should be cleared after reset
        {
            let guard2 = pool.acquire();
            guard2.with_lua(|lua| {
                let result: LuaValue = lua.load("return x").eval().unwrap();
                assert!(
                    matches!(result, LuaValue::Nil),
                    "Global 'x' should be nil after reset"
                );
            });
        }

        // Verify the same state was reused
        let stats = pool.stats();
        assert_eq!(stats.total_uses, 2);
    }

    #[test]
    fn test_preserved_globals_remain() {
        let pool = LuaPool::new(1);

        // First use: confirm preserved globals exist and add a user global
        {
            let guard = pool.acquire();
            guard.with_lua(|lua| {
                // math should be available
                let result: f64 = lua.load("return math.pi").eval().unwrap();
                assert!((result - std::f64::consts::PI).abs() < 0.0001);

                // Set user global
                lua.load("user_var = 'test'").exec().unwrap();
            });
        }

        // Second use: preserved globals should still exist, user global should be gone
        {
            let guard = pool.acquire();
            guard.with_lua(|lua| {
                // math should still be available
                let result: f64 = lua.load("return math.sqrt(4)").eval().unwrap();
                assert_eq!(result, 2.0);

                // string should be available
                let result: String = lua.load("return string.upper('hello')").eval().unwrap();
                assert_eq!(result, "HELLO");

                // table should be available
                let result: i32 = lua.load("local t = {1,2,3}; return #t").eval().unwrap();
                assert_eq!(result, 3);

                // User variable should be gone
                let result: LuaValue = lua.load("return user_var").eval().unwrap();
                assert!(matches!(result, LuaValue::Nil));
            });
        }
    }

    #[test]
    fn test_nested_tables_cleared() {
        let pool = LuaPool::new(1);

        // First use: create nested table structure
        {
            let guard = pool.acquire();
            guard.with_lua(|lua| {
                lua.load(
                    r#"
                    nested = {
                        level1 = {
                            level2 = {
                                value = "deep"
                            }
                        }
                    }
                    "#,
                )
                .exec()
                .unwrap();

                let result: String = lua
                    .load("return nested.level1.level2.value")
                    .eval()
                    .unwrap();
                assert_eq!(result, "deep");
            });
        }

        // Second use: nested table should be gone
        {
            let guard = pool.acquire();
            guard.with_lua(|lua| {
                let result: LuaValue = lua.load("return nested").eval().unwrap();
                assert!(matches!(result, LuaValue::Nil));
            });
        }
    }

    #[test]
    fn test_round_robin_distribution() {
        let pool = LuaPool::new(4);

        // Acquire and release states in sequence to test round-robin
        for i in 0..8 {
            let guard = pool.acquire();
            // Each acquisition should work
            guard.with_lua(|lua| {
                let result: i32 = lua.load(&format!("return {}", i)).eval().unwrap();
                assert_eq!(result, i);
            });
        }

        let stats = pool.stats();
        assert_eq!(stats.total_uses, 8);
        assert_eq!(stats.in_use, 0);
    }
}
