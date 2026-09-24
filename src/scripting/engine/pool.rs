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

/// Globals that survive [`LuaPool::reset_state`].
///
/// Also the list that gets snapshotted at static init and restored on reset,
/// so a script cannot hand the next tenant a replaced `pairs` or `string`.
const PRESERVED_GLOBALS: &[&str] = &[
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

/// Lua 5.4's incremental-collector defaults (LUAI_GCPAUSE, LUAI_GCMUL,
/// LUAI_GCSTEPSIZE), restored on reset.
const DEFAULT_GC_PAUSE: std::os::raw::c_int = 200;
const DEFAULT_GC_STEP_MUL: std::os::raw::c_int = 100;
const DEFAULT_GC_STEP_SIZE: std::os::raw::c_int = 13;

/// One shared library table behind its read-only proxy.
struct SharedTable {
    proxy: mlua::Table,
    backing: mlua::Table,
    /// The backing table's contents at static init.
    pristine: Vec<(LuaValue, LuaValue)>,
    /// Set when the engine wrote through the proxy while unlocked.
    dirty: Arc<AtomicBool>,
}

/// App data: every proxied table of a pooled state.
struct SharedTables(Vec<SharedTable>);

/// App data marker: present only while the engine itself writes per-request
/// fields into a shared table (see [`with_shared_tables_unlocked`]).
struct SharedTablesUnlocked;

/// Run engine-side setup that writes per-request fields (`solidb.auth`,
/// `solidb.log`, ...) into the proxied shared tables of a pooled state.
///
/// Script code must never run inside `f`: while the marker is set, any write
/// through a proxy is accepted.
pub(crate) fn with_shared_tables_unlocked<R>(lua: &Lua, f: impl FnOnce() -> R) -> R {
    struct Relock<'a>(&'a Lua);
    impl Drop for Relock<'_> {
        fn drop(&mut self) {
            self.0.remove_app_data::<SharedTablesUnlocked>();
        }
    }
    lua.set_app_data(SharedTablesUnlocked);
    let _relock = Relock(lua);
    f()
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
        if skip_reset {
            tracing::warn!(
                "Lua fast mode (skip_reset) is enabled: globals persist across requests \
                 in pooled states. Only use with trusted, stateless scripts."
            );
        }
        let states = (0..size)
            .map(|_| {
                let lua = Lua::new();
                Self::apply_memory_limit(&lua);
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
    ///
    /// Pool size can be overridden with `SOLIDB_LUA_POOL_SIZE` env var.
    /// Fast mode (skip reset) can be enabled with `SOLIDB_LUA_FAST_MODE=1`.
    pub fn with_default_size() -> Self {
        let size = std::env::var("SOLIDB_LUA_POOL_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|p| p.get())
                    .unwrap_or(4)
            })
            .max(4); // Minimum 4 states

        // Check environment variable for fast mode
        let fast_mode = std::env::var("SOLIDB_LUA_FAST_MODE")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false)
            && std::env::var("SOLIDB_LUA_FAST_MODE_UNSAFE")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
        if std::env::var("SOLIDB_LUA_FAST_MODE").is_ok() && !fast_mode {
            tracing::warn!(
                "SOLIDB_LUA_FAST_MODE ignored without SOLIDB_LUA_FAST_MODE_UNSAFE=1 \
                 (cross-request Lua state leak)"
            );
        }

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

        if let Some(guard) = self.try_acquire_from(start) {
            return guard;
        }

        // All states busy: wait for one, yielding the thread between
        // attempts. Async callers should poll `try_acquire` with a deadline
        // instead of blocking here.
        loop {
            if let Some(guard) = self.try_acquire_from(start) {
                return guard;
            }
            std::thread::yield_now();
        }
    }

    /// Acquire a state if one is free right now, without waiting.
    pub fn try_acquire(&self) -> Option<PoolGuard> {
        let start = self.next_index.fetch_add(1, Ordering::Relaxed) % self.size;
        self.try_acquire_from(start)
    }

    /// One lock-free pass over the states starting at `start`.
    fn try_acquire_from(&self, start: usize) -> Option<PoolGuard> {
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
                return Some(PoolGuard {
                    state: state.clone(),
                    index: idx,
                    skip_reset: self.skip_reset,
                });
            }
        }
        None
    }

    /// Cap a state's allocator so a script with an allocation loop OOMs its
    /// own request instead of the server. Default 64 MB; override with
    /// `SOLIDB_LUA_MEMORY_LIMIT_MB` (0 disables the limit).
    pub(crate) fn apply_memory_limit(lua: &Lua) {
        let limit_mb = std::env::var("SOLIDB_LUA_MEMORY_LIMIT_MB")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(64);
        if limit_mb > 0 {
            if let Err(e) = lua.set_memory_limit(limit_mb * 1024 * 1024) {
                tracing::warn!("Failed to set Lua memory limit: {}", e);
            }
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
        let status_fn = crate::scripting::response::create_status_function(lua)
            .map_err(|e| DbError::InternalError(format!("Failed to create status: {}", e)))?;
        solidb
            .set("status", status_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set status: {}", e)))?;
        let header_fn = crate::scripting::response::create_header_function(lua)
            .map_err(|e| DbError::InternalError(format!("Failed to create header: {}", e)))?;
        solidb
            .set("header", header_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set header: {}", e)))?;

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
        // The `response` global: json / html / redirect / file / cors.
        let response = crate::scripting::response::create_response_table(lua)
            .map_err(|e| DbError::InternalError(format!("Failed to create response: {}", e)))?;

        // Set globals
        globals
            .set("solidb", solidb)
            .map_err(|e| DbError::InternalError(format!("Failed to set solidb global: {}", e)))?;
        globals
            .set("response", response)
            .map_err(|e| DbError::InternalError(format!("Failed to set response global: {}", e)))?;

        // Lock the shared string metatable, put every shared library table
        // behind a read-only proxy, and snapshot the globals that survive a
        // reset. All three close cross-tenant leaks through a pooled state
        // (SEC-157, audit C3). The proxies must exist before the snapshot so
        // the snapshot restores the proxies, not the writable tables.
        Self::protect_shared_metatables(lua);
        Self::install_shared_table_proxies(lua);
        Self::snapshot_preserved_globals(lua);

        // Mark state as having static globals initialized
        globals
            .set("__solidb_static_initialized", true)
            .map_err(|e| {
                DbError::InternalError(format!("Failed to set static initialized marker: {}", e))
            })?;

        Ok(())
    }

    /// Registry key holding the pristine values of the globals that a reset
    /// preserves.
    const PRISTINE_GLOBALS_KEY: &'static str = "__solidb_pristine_globals";

    /// Make the string metatable unreachable from script code.
    ///
    /// Strings share one metatable across the whole Lua state, and it is the
    /// one metatable a sandboxed script can still reach: `debug` is removed,
    /// but `getmetatable("")` is not. `reset_state` nils user globals and
    /// never touched metatables, so
    /// `getmetatable("").__index = function(s, k) ... end` installed by one
    /// tenant stayed on the pooled state and ran inside the *next* tenant's
    /// script on any string method call — arbitrary code with that tenant's
    /// database handle.
    ///
    /// Setting `__metatable` is the standard Lua answer: `getmetatable("")`
    /// now returns this marker string instead of the table, and
    /// `setmetatable` on a string raises "cannot change a protected
    /// metatable". Cheaper and more robust than trying to scrub the table
    /// after the fact, which would itself have to call globals a script may
    /// have replaced.
    fn protect_shared_metatables(lua: &Lua) {
        // `Lua::load` is the Rust-side loader; it does not depend on the
        // `load` global, which the sandbox removes.
        let chunk = r#"
            local getmt, setmt, str = ...
            local mt = getmt("")
            if type(mt) == "table" and rawget(mt, "__metatable") == nil then
                if rawget(mt, "__index") == nil then
                    rawset(mt, "__index", str)
                end
                rawset(mt, "__metatable", "protected: string metatable")
            end
            return true
        "#;
        let string_table: LuaValue = lua.globals().get("string").unwrap_or(LuaValue::Nil);
        let getmt: LuaValue = lua.globals().get("getmetatable").unwrap_or(LuaValue::Nil);
        let setmt: LuaValue = lua.globals().get("setmetatable").unwrap_or(LuaValue::Nil);
        // Passed as arguments rather than read from globals inside the chunk:
        // this runs before any script, so the globals are still pristine, and
        // the chunk stays independent of them.
        if let Err(e) = lua.load(chunk).call::<bool>((getmt, setmt, string_table)) {
            tracing::warn!("Failed to protect the string metatable: {}", e);
        }
    }

    /// Replace every preserved library table (`string`, `table`, `crypto`,
    /// `json`, `solidb`, ...) with a read-only proxy.
    ///
    /// Restoring a preserved global by reference (SEC-157) put back *which*
    /// table `crypto` names, not what is inside it: one tenant's
    /// `crypto.verify_password = function() return true end` ran inside the
    /// next tenant's login script, and `string.__loot = db` handed the next
    /// request's caller the previous tenant's database handle (audit C3).
    ///
    /// Each proxy is an empty table whose metatable reads through to the
    /// real ("backing") table, refuses writes, iterates the backing table for
    /// `pairs`, and is locked with `__metatable` so a script can neither see
    /// the backing table nor detach the metatable. Nested tables are proxied
    /// the same way. `rawset` on a proxy still writes to the proxy itself;
    /// [`Self::restore_shared_tables`] empties every proxy on reset.
    ///
    /// The per-request fields of `solidb` (`auth`, `log`, `env`, ...) are
    /// written by the engine through `Table::set`, which goes through
    /// `__newindex`: those writes are allowed only while the engine holds
    /// [`with_shared_tables_unlocked`], and they mark the table dirty so the
    /// reset restores it to its static-init contents.
    ///
    /// The string metatable's `__index` keeps pointing at the backing
    /// `string` table, so `("x"):upper()` is unaffected.
    fn install_shared_table_proxies(lua: &Lua) {
        if let Err(e) = Self::install_shared_table_proxies_inner(lua) {
            tracing::warn!("Failed to install read-only shared-table proxies: {}", e);
        }
    }

    fn install_shared_table_proxies_inner(lua: &Lua) -> mlua::Result<()> {
        let globals = lua.globals();
        let setmt: mlua::Function = globals.get("setmetatable")?;
        let next_fn: mlua::Function = globals.get("next")?;
        // Built with the pristine `setmetatable`/`next`, passed in as
        // arguments: nothing here reads a global a script could replace.
        let make_proxy: mlua::Function = lua
            .load(
                r#"
                local backing, newindex, setmetatable, next = ...
                local proxy = {}
                local function iter(_, k) return next(backing, k) end
                setmetatable(proxy, {
                    __index = backing,
                    __newindex = newindex,
                    __pairs = function(t) return iter, t, nil end,
                    __len = function() return #backing end,
                    __metatable = "protected: shared library table",
                })
                return proxy
                "#,
            )
            .into_function()?;

        let mut shared = Vec::new();
        let mut seen: std::collections::HashMap<usize, mlua::Table> =
            std::collections::HashMap::new();
        for name in PRESERVED_GLOBALS {
            if *name == "_G" {
                continue;
            }
            if let Ok(LuaValue::Table(backing)) = globals.raw_get::<LuaValue>(*name) {
                let proxy = Self::wrap_shared_table(
                    lua,
                    &make_proxy,
                    &setmt,
                    &next_fn,
                    backing,
                    &mut seen,
                    &mut shared,
                )?;
                globals.raw_set(*name, proxy)?;
            }
        }
        lua.set_app_data(SharedTables(shared));
        Ok(())
    }

    fn wrap_shared_table(
        lua: &Lua,
        make_proxy: &mlua::Function,
        setmt: &mlua::Function,
        next_fn: &mlua::Function,
        backing: mlua::Table,
        seen: &mut std::collections::HashMap<usize, mlua::Table>,
        shared: &mut Vec<SharedTable>,
    ) -> mlua::Result<mlua::Table> {
        let id = backing.to_pointer() as usize;
        if let Some(proxy) = seen.get(&id) {
            return Ok(proxy.clone());
        }
        let dirty = Arc::new(AtomicBool::new(false));
        let newindex = {
            let backing = backing.clone();
            let dirty = dirty.clone();
            lua.create_function(
                move |lua, (_proxy, key, value): (LuaValue, LuaValue, LuaValue)| {
                    if lua.app_data_ref::<SharedTablesUnlocked>().is_none() {
                        let field = match &key {
                            LuaValue::String(s) => s.to_string_lossy().to_string(),
                            other => other.type_name().to_string(),
                        };
                        return Err(mlua::Error::RuntimeError(format!(
                            "attempt to modify read-only shared table (field '{}')",
                            field
                        )));
                    }
                    dirty.store(true, Ordering::Relaxed);
                    backing.raw_set(key, value)
                },
            )?
        };
        let proxy: mlua::Table =
            make_proxy.call((backing.clone(), newindex, setmt.clone(), next_fn.clone()))?;
        // Recorded before recursing so a cycle resolves to this proxy.
        seen.insert(id, proxy.clone());

        // Nested tables (none of the stdlib ones, but a namespace may grow
        // one) are proxied too, in place inside the backing table.
        let nested: Vec<(LuaValue, mlua::Table)> = backing
            .pairs::<LuaValue, LuaValue>()
            .filter_map(|r| match r {
                Ok((k, LuaValue::Table(t))) => Some((k, t)),
                _ => None,
            })
            .collect();
        for (key, table) in nested {
            let nested_proxy =
                Self::wrap_shared_table(lua, make_proxy, setmt, next_fn, table, seen, shared)?;
            backing.raw_set(key, nested_proxy)?;
        }

        let pristine: Vec<(LuaValue, LuaValue)> = backing
            .pairs::<LuaValue, LuaValue>()
            .filter_map(|r| r.ok())
            .collect();
        shared.push(SharedTable {
            proxy: proxy.clone(),
            backing,
            pristine,
            dirty,
        });
        Ok(proxy)
    }

    /// Put every shared library table back to its static-init contents.
    ///
    /// Proxies are emptied (anything in one came from `rawset`), and a
    /// backing table the engine wrote to during the request — the per-request
    /// `solidb.*` fields — is rebuilt from its snapshot, which drops those
    /// fields generically rather than from a list that has to be kept in
    /// step with the setup code.
    fn restore_shared_tables(lua: &Lua) {
        let Some(tables) = lua.app_data_ref::<SharedTables>() else {
            return;
        };
        for t in tables.0.iter() {
            if !t.proxy.is_empty() {
                let _ = t.proxy.clear();
            }
            if t.dirty.swap(false, Ordering::Relaxed) {
                let keys: Vec<LuaValue> = t
                    .backing
                    .pairs::<LuaValue, LuaValue>()
                    .filter_map(|r| r.ok().map(|(k, _)| k))
                    .collect();
                for k in keys {
                    let _ = t.backing.raw_set(k, LuaValue::Nil);
                }
                for (k, v) in &t.pristine {
                    let _ = t.backing.raw_set(k.clone(), v.clone());
                }
            }
        }
    }

    /// Store the pristine value of every preserved global in the Lua registry.
    ///
    /// `reset_state` keeps these globals rather than nilling them, which also
    /// means it keeps a script's *replacement* for one: `pairs = function() end`
    /// or `string = {}` set by one tenant was inherited by the next. The
    /// registry is not reachable from script code, so restoring from it is
    /// sound where re-reading the globals would not be.
    fn snapshot_preserved_globals(lua: &Lua) {
        let globals = lua.globals();
        let snapshot = match lua.create_table() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Failed to snapshot preserved globals: {}", e);
                return;
            }
        };
        for name in PRESERVED_GLOBALS {
            // `_G` is the globals table itself; storing it would pin a cycle
            // and it cannot be meaningfully restored anyway.
            if *name == "_G" {
                continue;
            }
            if let Ok(value) = globals.get::<LuaValue>(*name) {
                if !matches!(value, LuaValue::Nil) {
                    let _ = snapshot.set(*name, value);
                }
            }
        }
        if let Err(e) = lua.set_named_registry_value(Self::PRISTINE_GLOBALS_KEY, snapshot) {
            tracing::warn!("Failed to store preserved-global snapshot: {}", e);
        }
    }

    /// Put the preserved globals back to the values captured at static init.
    fn restore_preserved_globals(lua: &Lua) {
        let Ok(snapshot) = lua.named_registry_value::<mlua::Table>(Self::PRISTINE_GLOBALS_KEY)
        else {
            return;
        };
        let globals = lua.globals();
        // `_G` is not in the snapshot (it would be a cycle), but a script
        // can still reassign it: `_G = {loot = db}` survived for the next
        // tenant. It always names the globals table itself.
        let _ = globals.raw_set("_G", globals.clone());
        for name in PRESERVED_GLOBALS {
            if *name == "_G" {
                continue;
            }
            if let Ok(value) = snapshot.get::<LuaValue>(*name) {
                if !matches!(value, LuaValue::Nil) {
                    let _ = globals.raw_set(*name, value);
                }
            }
        }
    }

    /// Reset a Lua state for reuse.
    ///
    /// This clears user-defined globals while preserving:
    /// - Standard Lua globals (pairs, ipairs, math, string, table, etc.)
    /// - Static globals initialized by the pool (crypto, time, json, solidb, response)
    fn reset_state(lua: &Lua) {
        let globals = lua.globals();

        // A metatable on `_G` (`setmetatable(_G, {__index = ..., __newindex
        // = ...})`) would otherwise stay on the state, see every global the
        // next tenant reads or assigns, and run during this very reset.
        // Rust's `set_metatable` ignores `__metatable` protection.
        let _ = globals.set_metatable(None);

        // Collect keys to remove (these are per-request globals like db,
        // request, context). Every key type: a `String`-typed iteration
        // silently skipped `_G[1] = db` or `_G[true] = db`.
        let mut to_remove = Vec::new();

        let pairs = globals.pairs::<LuaValue, LuaValue>();
        for (key, _) in pairs.flatten() {
            let preserved = match &key {
                LuaValue::String(s) => {
                    let name = s.to_string_lossy();
                    PRESERVED_GLOBALS.iter().any(|p| *p == &*name)
                }
                _ => false,
            };
            if !preserved {
                to_remove.push(key);
            }
        }

        // Remove non-preserved globals (db, request, context, etc.)
        for key in to_remove {
            let _ = globals.raw_set(key, LuaValue::Nil);
        }

        // Preserving a global by name is not the same as preserving its
        // value: a script that assigned `pairs = ...` or `string = {}` left
        // its replacement in place for the next tenant on this state. Put the
        // pristine values back from the registry snapshot (SEC-157).
        Self::restore_preserved_globals(lua);

        // The contents of the shared tables, including the per-request
        // `solidb.*` fields (auth, log, env, file functions, ai, streams,
        // stats), which setup_request_globals writes again (audit C3).
        Self::restore_shared_tables(lua);

        // Per-request identity and response state live in app data, not in
        // globals. Drop them so a script whose setup is skipped (nothing in
        // it looked like it needed globals) runs as nobody, not as the
        // previous caller.
        lua.remove_app_data::<crate::scripting::engine::globals::LuaCaller>();
        lua.remove_app_data::<crate::scripting::types::ScriptDbName>();
        lua.remove_app_data::<crate::scripting::response::ResponseOverrides>();
        lua.remove_app_data::<SharedTablesUnlocked>();

        // `collectgarbage("stop")` or a tuned/generational collector would
        // otherwise persist into the next tenant's request on this state.
        lua.gc_restart();
        let _ = lua.gc_inc(DEFAULT_GC_PAUSE, DEFAULT_GC_STEP_MUL, DEFAULT_GC_STEP_SIZE);
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
                            .load(format!("return {} + 1", i))
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
                    .load(format!("return {}", name))
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
                let result: i32 = lua.load(format!("return {}", i)).eval().unwrap();
                assert_eq!(result, i);
            });
        }

        let stats = pool.stats();
        assert_eq!(stats.total_uses, 8);
        assert_eq!(stats.in_use, 0);
    }

    /// SEC-157: one tenant's script must not be able to leave a hook on the
    /// shared string metatable for the next tenant on the same pooled state.
    #[test]
    fn string_metatable_is_not_reachable_from_script_code() {
        let pool = LuaPool::new(1);
        let guard = pool.acquire();
        guard.with_lua(|lua| {
            // `getmetatable("")` now yields the protection marker, not the
            // table, so there is no `__index` to overwrite.
            let mt_type: String = lua
                .load(r#"return type(getmetatable(""))"#)
                .call(())
                .expect("getmetatable call");
            assert_eq!(mt_type, "string", "string metatable must be protected");

            // And the poisoning attempt itself fails rather than silently
            // installing a hook.
            let poisoned: bool = lua
                .load(
                    r#"
                    local ok = pcall(function()
                        getmetatable("").__index = function() return "pwned" end
                    end)
                    return ok
                    "#,
                )
                .call(())
                .expect("poison attempt");
            assert!(!poisoned, "poisoning the string metatable must fail");

            // String methods still work normally.
            let upper: String = lua
                .load(r#"return ("abc"):upper()"#)
                .call(())
                .expect("string method");
            assert_eq!(upper, "ABC");
        });
    }

    /// A replaced preserved global must not survive into the next borrow.
    #[test]
    fn replaced_preserved_globals_are_restored_on_reset() {
        let pool = LuaPool::new(1);
        {
            let guard = pool.acquire();
            guard.with_lua(|lua| {
                lua.load(r#"string = {}; pairs = function() end"#)
                    .exec()
                    .expect("clobber globals");
            });
        }
        let guard = pool.acquire();
        guard.with_lua(|lua| {
            let has_upper: bool = lua
                .load(r#"return type(string) == "table" and type(string.upper) == "function""#)
                .call(())
                .expect("check string");
            assert!(has_upper, "`string` must be restored for the next tenant");
            let pairs_ok: bool = lua
                .load(r#"local n = 0; for _ in pairs({1,2}) do n = n + 1 end; return n == 2"#)
                .call(())
                .expect("check pairs");
            assert!(pairs_ok, "`pairs` must be restored for the next tenant");
        });
    }

    /// Audit C3: a tenant must not be able to change a field of a shared
    /// library table in a way the next tenant on the same state inherits.
    #[test]
    fn shared_table_fields_do_not_leak_across_borrows() {
        let pool = LuaPool::new(1);
        {
            let guard = pool.acquire();
            guard.with_lua(|lua| {
                // Ordinary assignment is refused outright.
                let refused: bool = lua
                    .load(
                        r#"
                        local all_refused = true
                        for _, f in ipairs({
                            function() crypto.verify_password = function() return true end end,
                            function() json.encode = function() return "evil" end end,
                            function() string.__loot = {} end,
                            function() table.insert = function() end end,
                            function() solidb.now = function() return 0 end end,
                            function() setmetatable(crypto, nil) end,
                        }) do
                            if pcall(f) then all_refused = false end
                        end
                        return all_refused
                        "#,
                    )
                    .call(())
                    .expect("mutation attempts");
                assert!(refused, "writes to shared tables must raise");

                // `rawset` bypasses __newindex and lands on the proxy; it
                // only affects this borrow.
                lua.load(
                    r#"
                    rawset(crypto, "verify_password", function() return true end)
                    rawset(string, "__loot", "secret")
                    rawset(table, "insert", function() end)
                    collectgarbage("stop")
                    "#,
                )
                .exec()
                .expect("rawset on proxies");
                let leaked: String = lua
                    .load(r#"return string.__loot"#)
                    .call(())
                    .expect("same-borrow read");
                assert_eq!(leaked, "secret");
            });
        }
        let guard = pool.acquire();
        guard.with_lua(|lua| {
            let clean: bool = lua
                .load(
                    r#"
                    return string.__loot == nil
                        and type(crypto.verify_password) == "function"
                        and rawget(crypto, "verify_password") == nil
                        and rawget(table, "insert") == nil
                        and json.decode(json.encode({a = 1})).a == 1
                    "#,
                )
                .call(())
                .expect("check originals");
            assert!(clean, "next tenant must see the original shared tables");

            let t: i64 = lua
                .load(r#"local t = {}; table.insert(t, 5); return t[1]"#)
                .call(())
                .expect("table.insert");
            assert_eq!(t, 5);

            let running: bool = lua
                .load(r#"return collectgarbage("isrunning")"#)
                .call(())
                .expect("gc state");
            assert!(running, "collectgarbage(\"stop\") must not persist");

            // String methods and `pairs` over a proxied library still work.
            let ok: bool = lua
                .load(
                    r#"
                    local n = 0
                    for k, v in pairs(string) do n = n + 1 end
                    return ("abc"):upper() == "ABC" and string.format("%d", 3) == "3" and n > 10
                    "#,
                )
                .call(())
                .expect("string methods");
            assert!(ok);
        });
    }

    /// Non-string global keys and a metatable on `_G` must not survive.
    #[test]
    fn g_metatable_and_non_string_globals_are_cleared() {
        let pool = LuaPool::new(1);
        {
            let guard = pool.acquire();
            guard.with_lua(|lua| {
                lua.load(
                    r#"
                    _G[1] = "loot"
                    _G[true] = "loot"
                    setmetatable(_G, { __index = function() return "hooked" end })
                    "#,
                )
                .exec()
                .unwrap();
            });
        }
        let guard = pool.acquire();
        guard.with_lua(|lua| {
            let clean: bool = lua
                .load(r#"return rawget(_G, 1) == nil and rawget(_G, true) == nil and undefined_name == nil and getmetatable(_G) == nil"#)
                .call(())
                .unwrap();
            assert!(clean);
        });
    }

    /// Engine-side per-request writes to `solidb` are allowed while
    /// unlocked, and dropped by the reset.
    #[test]
    fn unlocked_solidb_fields_are_cleared_on_reset() {
        let pool = LuaPool::new(1);
        {
            let guard = pool.acquire();
            guard.with_lua(|lua| {
                let solidb: mlua::Table = lua.globals().get("solidb").unwrap();
                with_shared_tables_unlocked(lua, || solidb.set("auth", "tenant-a"))
                    .expect("unlocked write");
                assert!(solidb.set("auth", "script").is_err(), "locked again");
                let seen: String = lua.load("return solidb.auth").call(()).unwrap();
                assert_eq!(seen, "tenant-a");
            });
        }
        let guard = pool.acquire();
        guard.with_lua(|lua| {
            let gone: bool = lua
                .load("return solidb.auth == nil and type(solidb.now) == 'function'")
                .call(())
                .unwrap();
            assert!(gone);
        });
    }
}

#[cfg(test)]
mod saturation_tests {
    use super::*;

    /// With every state held, `try_acquire` reports it instead of spinning.
    #[test]
    fn try_acquire_reports_saturation() {
        let pool = LuaPool::new(2);
        let g1 = pool.try_acquire().expect("first state");
        let g2 = pool.try_acquire().expect("second state");
        assert!(pool.try_acquire().is_none());
        drop(g1);
        assert!(pool.try_acquire().is_some());
        drop(g2);
    }
}
