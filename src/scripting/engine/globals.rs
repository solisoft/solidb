use mlua::{Lua, Value as LuaValue};
use serde_json::Value as JsonValue;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::error::DbError;
use crate::scripting::conversion::{json_to_lua, lua_to_json_value, matches_filter};
use crate::scripting::dev_tools::*;
use crate::scripting::error_handling::*;
use crate::scripting::file_handling::*;
use crate::scripting::http_helpers::*;
use crate::scripting::types::ScriptContext;
use crate::scripting::validation::*;
use crate::scripting::{ai_bindings, auth, lua_globals};
use crate::sdbql::parser::parse;
use crate::storage::StorageEngine;
use crate::stream::StreamManager;
use crate::QueryExecutor;

use super::ScriptEngine;

/// Environment variable cache with TTL
/// Key: db_name, Value: (timestamp, env_vars)
use dashmap::DashMap;
use once_cell::sync::Lazy;

/// Type alias for the environment cache entry: (timestamp, env_vars)
type EnvCacheEntry = (Instant, std::collections::HashMap<String, String>);

static ENV_CACHE: Lazy<DashMap<String, EnvCacheEntry>> = Lazy::new(DashMap::new);

const ENV_CACHE_TTL: Duration = Duration::from_secs(5);

/// Global cache for parsed SDBQL ASTs to avoid re-parsing the same query strings.
/// Key: query string, Value: parsed Query AST
static QUERY_AST_CACHE: Lazy<DashMap<String, Arc<crate::sdbql::ast::Query>>> =
    Lazy::new(|| DashMap::with_capacity(256));

/// Entries kept in each of the two global Lua query caches.
///
/// Both are keyed by data a script chooses, so without a ceiling a single
/// request could grow them without limit: `for i = 1, 1e8 do
/// db:query("RETURN " .. i) end` adds a permanent AST entry per iteration.
/// These allocations are Rust-side and therefore *outside* the 64 MB
/// per-state Lua memory limit — the script stays well within its own budget
/// while exhausting the host's.
///
/// The bound is deliberately generous: this is a safety ceiling, not a
/// tuning knob. Real workloads have a small set of distinct query strings.
const MAX_LUA_QUERY_CACHE_ENTRIES: usize = 4096;

/// Drop everything from a global cache once it grows past the ceiling.
///
/// Clearing wholesale rather than evicting least-recently-used: `DashMap` has
/// no recency information, an LRU would need a second index and a lock on the
/// hot path, and the cost of a rare full rebuild is bounded (these are
/// re-derivable caches with a 100 ms TTL on the result side).
fn enforce_cache_ceiling<K, V>(cache: &DashMap<K, V>, name: &str)
where
    K: std::hash::Hash + Eq,
{
    if cache.len() > MAX_LUA_QUERY_CACHE_ENTRIES {
        tracing::warn!(
            "{} exceeded {} entries; clearing. A script is issuing queries with \
             unbounded distinct text or bind variables.",
            name,
            MAX_LUA_QUERY_CACHE_ENTRIES
        );
        cache.clear();
    }
}

/// Parse a query with caching. Returns a cached AST clone or parses and caches.
fn parse_cached(query: &str) -> crate::error::DbResult<Arc<crate::sdbql::ast::Query>> {
    if let Some(ast) = QUERY_AST_CACHE.get(query) {
        return Ok(Arc::clone(ast.value()));
    }
    let ast = Arc::new(parse(query)?);
    enforce_cache_ceiling(&QUERY_AST_CACHE, "Lua query AST cache");
    QUERY_AST_CACHE.insert(query.to_string(), Arc::clone(&ast));
    Ok(ast)
}

/// Sync-friendly query result cache for Lua db:query() hot path.
/// Key: u64 hash (zero-alloc), Value: (cached_at, json_values, json_string)
type QueryResultEntry = (Instant, Arc<Vec<JsonValue>>, Arc<String>);
/// The caller a Lua state is currently serving, kept in mlua app data.
///
/// Not a Lua global (a script could overwrite it) and not captured by the
/// `db` closures (a pooled state keeps those closures across requests from
/// different callers — see `setup_request_globals_selective`). Every write
/// through a `db` handle reads it at call time.
#[derive(Clone)]
pub(crate) struct LuaCaller {
    pub actor: crate::storage::WriteActor,
    pub principal: crate::sdbql::QueryPrincipal,
}

impl LuaCaller {
    fn from_context(context: &ScriptContext) -> Self {
        Self {
            actor: context.write_actor(),
            principal: context.query_principal(),
        }
    }

    /// Nothing installed for this state: fail closed, as an anonymous caller.
    fn fail_closed() -> Self {
        Self {
            actor: crate::storage::WriteActor::client(false),
            principal: crate::sdbql::QueryPrincipal::anonymous(),
        }
    }
}

fn caller_of(lua: &Lua) -> LuaCaller {
    lua.app_data_ref::<LuaCaller>()
        .map(|c| c.clone())
        .unwrap_or_else(LuaCaller::fail_closed)
}

static LUA_QUERY_CACHE: Lazy<DashMap<u64, QueryResultEntry>> =
    Lazy::new(|| DashMap::with_capacity(1024));

const LUA_QUERY_CACHE_TTL: Duration = Duration::from_millis(100);

/// Compute a u64 cache key from db_name + query + bind vars (zero-alloc).
fn query_cache_key(
    db_name: &str,
    query: &str,
    bind_vars_map: &std::collections::HashMap<String, JsonValue>,
    principal: &crate::sdbql::QueryPrincipal,
) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    db_name.hash(&mut hasher);
    query.hash(&mut hasher);
    // Row policies make the same query answer differently per caller.
    principal.user.hash(&mut hasher);
    (principal.can_read, principal.can_write, principal.can_admin).hash(&mut hasher);
    if !bind_vars_map.is_empty() {
        let mut sorted: Vec<_> = bind_vars_map.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        for (k, v) in &sorted {
            k.hash(&mut hasher);
            v.to_string().hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Execute a query and populate the cache.
fn query_execute_and_cache(
    storage: &Arc<StorageEngine>,
    db_name: &str,
    query: &str,
    bind_vars_map: std::collections::HashMap<String, JsonValue>,
    cache_key: u64,
    principal: &crate::sdbql::QueryPrincipal,
) -> Result<(Arc<Vec<JsonValue>>, Arc<String>), crate::error::DbError> {
    let query_ast = parse_cached(query)?;

    // The caller's principal, so a script's INSERT/UPDATE/REMOVE meets the
    // same write tiers and row policies as the same query on `/cursor`.
    // Without one the executor writes as the server itself.
    let executor = if bind_vars_map.is_empty() {
        QueryExecutor::with_database(storage, db_name.to_string())
    } else {
        QueryExecutor::with_database_and_bind_vars(storage, db_name.to_string(), bind_vars_map)
    }
    .with_principal(principal.clone());

    let results = executor.execute(&query_ast)?;
    let json_str = serde_json::to_string(&results).unwrap_or_else(|_| "[]".to_string());
    let results = Arc::new(results);
    let json_str = Arc::new(json_str);

    // A mutation's result is not a cacheable answer: replaying it would skip
    // the write.
    if !query_ast.has_mutations() {
        enforce_cache_ceiling(&LUA_QUERY_CACHE, "Lua query result cache");
        LUA_QUERY_CACHE.insert(
            cache_key,
            (Instant::now(), Arc::clone(&results), Arc::clone(&json_str)),
        );
    }

    Ok((results, json_str))
}

/// Get or execute a read-only query with caching. Returns Vec<JsonValue>.
fn query_cached(
    storage: &Arc<StorageEngine>,
    db_name: &str,
    query: &str,
    bind_vars_map: std::collections::HashMap<String, JsonValue>,
    principal: &crate::sdbql::QueryPrincipal,
) -> Result<Arc<Vec<JsonValue>>, crate::error::DbError> {
    let cache_key = query_cache_key(db_name, query, &bind_vars_map, principal);

    if let Some(entry) = LUA_QUERY_CACHE.get(&cache_key) {
        let (cached_at, results, _) = entry.value();
        if cached_at.elapsed() < LUA_QUERY_CACHE_TTL {
            return Ok(Arc::clone(results));
        }
    }

    let (results, _) =
        query_execute_and_cache(storage, db_name, query, bind_vars_map, cache_key, principal)?;
    Ok(results)
}

/// Get or execute a read-only query with caching. Returns pre-serialized JSON string.
fn query_cached_json(
    storage: &Arc<StorageEngine>,
    db_name: &str,
    query: &str,
    bind_vars_map: std::collections::HashMap<String, JsonValue>,
    principal: &crate::sdbql::QueryPrincipal,
) -> Result<Arc<String>, crate::error::DbError> {
    let cache_key = query_cache_key(db_name, query, &bind_vars_map, principal);

    if let Some(entry) = LUA_QUERY_CACHE.get(&cache_key) {
        let (cached_at, _, json_str) = entry.value();
        if cached_at.elapsed() < LUA_QUERY_CACHE_TTL {
            return Ok(Arc::clone(json_str));
        }
    }

    let (_, json_str) =
        query_execute_and_cache(storage, db_name, query, bind_vars_map, cache_key, principal)?;
    Ok(json_str)
}

/// Flags indicating which globals a script needs, derived from code analysis.
/// Cached per script to avoid re-scanning on every request.
#[derive(Clone, Copy, Default)]
pub struct ScriptNeeds {
    pub db: bool,
    pub request: bool,
    pub solidb_log: bool,
    pub solidb_stats: bool,
    pub solidb_auth: bool,
    pub solidb_env: bool,
    pub solidb_file: bool,
    pub solidb_ai: bool,
    pub solidb_stream: bool,
}

impl ScriptNeeds {
    /// Analyze script code to determine which globals it references.
    pub fn analyze(code: &str) -> Self {
        Self {
            db: code.contains("db.") || code.contains("db:"),
            request: code.contains("request") || code.contains("context"),
            solidb_log: code.contains("solidb.log") || code.contains("solidb:log"),
            solidb_stats: code.contains("solidb.stats") || code.contains("solidb:stats"),
            solidb_auth: code.contains("solidb.auth") || code.contains("auth"),
            solidb_env: code.contains("solidb.env"),
            solidb_file: code.contains("solidb.file")
                || code.contains("solidb.upload")
                || code.contains("solidb.image"),
            solidb_ai: code.contains("solidb.ai"),
            solidb_stream: code.contains("solidb.stream"),
        }
    }

    /// Returns true if any globals need setup at all.
    pub fn any(&self) -> bool {
        self.db
            || self.request
            || self.solidb_log
            || self.solidb_stats
            || self.solidb_auth
            || self.solidb_env
            || self.solidb_file
            || self.solidb_ai
            || self.solidb_stream
    }
}

/// Setup only per-request globals (fast path for pooled states with static globals)
///
/// This function is called on each request when the pool state already has
/// static globals initialized. Only sets up globals the script actually references.
pub fn setup_request_globals(
    engine: &ScriptEngine,
    lua: &Lua,
    db_name: &str,
    context: &ScriptContext,
    script_info: Option<(&str, &str)>,
) -> Result<(), DbError> {
    setup_request_globals_selective(engine, lua, db_name, context, script_info, None)
}

/// Setup per-request globals, optionally filtered by what the script needs.
pub fn setup_request_globals_selective(
    engine: &ScriptEngine,
    lua: &Lua,
    db_name: &str,
    context: &ScriptContext,
    script_info: Option<(&str, &str)>,
    needs: Option<&ScriptNeeds>,
) -> Result<(), DbError> {
    // If no needs provided, set up everything (backwards compatible)
    let all = ScriptNeeds {
        db: true,
        request: true,
        solidb_log: true,
        solidb_stats: true,
        solidb_auth: true,
        solidb_env: true,
        solidb_file: true,
        solidb_ai: true,
        solidb_stream: true,
    };
    let needs = needs.unwrap_or(&all);

    // Early return if nothing needed
    if !needs.any() {
        return Ok(());
    }

    // Who this request writes as. Set before the fast path below: the `db`
    // closures survive across callers on a pooled state, and read this at
    // call time.
    lua.set_app_data(LuaCaller::from_context(context));
    crate::scripting::response::reset_overrides(lua);

    let globals = lua.globals();

    // Check if db-level globals are already set up for this db+script combo.
    // If so, only refresh per-request globals (request table + auth).
    let bound_key: String = globals.get("__solidb_bound_key").unwrap_or_default();
    let expected_key = format!("{}:{}", db_name, script_info.map(|(k, _)| k).unwrap_or(""));
    let already_bound = !bound_key.is_empty() && bound_key == expected_key;

    if already_bound {
        // Fast path: only refresh what changes per-request.
        //
        // Everything derived from `context.user` MUST be refreshed here. A
        // pooled Lua state is bound to `{db}:{script}`, not to a caller, so
        // whatever this branch leaves in place is inherited by the next
        // request that lands on the slot — and with round-robin selection,
        // that is eventually every caller of the script. `solidb.env` was
        // only populated in the full-setup branch below, behind an admin
        // check, so an admin request warmed the slot with provider API keys
        // and subsequent non-admin requests read them straight out of the
        // still-populated table, defeating the SEC-176 gate.
        if needs.request {
            setup_request_table(lua, context)?;
        }
        if needs.solidb_auth || needs.solidb_env {
            if let Ok(solidb) = globals.get::<mlua::Table>("solidb") {
                if needs.solidb_auth {
                    let auth_table = auth::create_auth_table(lua, &context.user).map_err(|e| {
                        DbError::InternalError(format!("Failed to create auth table: {}", e))
                    })?;
                    solidb.set("auth", auth_table).map_err(|e| {
                        DbError::InternalError(format!("Failed to set auth: {}", e))
                    })?;
                }
                if needs.solidb_env {
                    // Re-evaluated against *this* caller: replaces the table
                    // wholesale, with an empty one for a non-admin.
                    let allow_secrets =
                        context.user.authenticated && context.user.has_role("admin");
                    setup_env_table_cached(engine, lua, &solidb, db_name, allow_secrets)?;
                }
            }
        }
        return Ok(());
    }

    // Full setup: set up all needed globals and mark the pool state as bound
    let needs_solidb = needs.solidb_log
        || needs.solidb_stats
        || needs.solidb_auth
        || needs.solidb_env
        || needs.solidb_file
        || needs.solidb_ai
        || needs.solidb_stream;

    let solidb: Option<mlua::Table> = if needs_solidb {
        Some(
            globals
                .get("solidb")
                .map_err(|e| DbError::InternalError(format!("solidb table not found: {}", e)))?,
        )
    } else {
        None
    };

    if needs.solidb_log {
        if let Some(ref solidb) = solidb {
            setup_log_function(engine, lua, solidb, db_name, script_info)?;
        }
    }

    if needs.solidb_stats {
        if let Some(ref solidb) = solidb {
            setup_stats_function(lua, solidb, &engine.stats)?;
        }
    }

    if needs.solidb_auth {
        if let Some(ref solidb) = solidb {
            let auth_table = auth::create_auth_table(lua, &context.user).map_err(|e| {
                DbError::InternalError(format!("Failed to create auth table: {}", e))
            })?;
            solidb
                .set("auth", auth_table)
                .map_err(|e| DbError::InternalError(format!("Failed to set auth: {}", e)))?;
        }
    }

    if needs.solidb_env {
        if let Some(ref solidb) = solidb {
            let allow_secrets = context.user.authenticated && context.user.has_role("admin");
            setup_env_table_cached(engine, lua, solidb, db_name, allow_secrets)?;
        }
    }

    if needs.solidb_file {
        if let Some(ref solidb) = solidb {
            setup_file_functions(lua, solidb, &engine.storage, db_name)?;
        }
    }

    if needs.solidb_ai {
        if let Some(ref solidb) = solidb {
            let ai_table = ai_bindings::create_ai_table(lua, engine.storage.clone(), db_name)
                .map_err(|e| DbError::InternalError(format!("Failed to create AI table: {}", e)))?;
            solidb
                .set("ai", ai_table)
                .map_err(|e| DbError::InternalError(format!("Failed to set solidb.ai: {}", e)))?;
        }
    }

    if needs.solidb_stream {
        if let Some(ref solidb) = solidb {
            if let Some(stream_manager) = engine.stream_manager.clone() {
                setup_streams_table(lua, solidb, stream_manager)?;
            }
        }
    }

    if needs.db {
        setup_db_object(engine, lua, db_name)?;
    }

    if needs.request {
        setup_request_table(lua, context)?;
    }

    // Mark this pool state as bound to this db+script
    let _ = globals.set("__solidb_bound_key", expected_key);

    Ok(())
}

fn setup_log_function(
    engine: &ScriptEngine,
    lua: &Lua,
    solidb: &mlua::Table,
    db_name: &str,
    script_info: Option<(&str, &str)>,
) -> Result<(), DbError> {
    let storage_log = engine.storage.clone();
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

            let label = script_details
                .as_ref()
                .map(|(_, n)| n.as_str())
                .unwrap_or("Lua Script");
            tracing::info!("[{}] [{}] {}", db_log, label, msg);

            if let Some((sid, sname)) = &script_details {
                if let Ok(db) = storage_log.get_database(&db_log) {
                    let collection_res = db.get_collection("_logs");
                    let collection = match collection_res {
                        Ok(c) => Some(c),
                        Err(DbError::CollectionNotFound(_)) => {
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

    Ok(())
}

fn setup_stats_function(
    lua: &Lua,
    solidb: &mlua::Table,
    stats: &Arc<crate::scripting::types::ScriptStats>,
) -> Result<(), DbError> {
    let stats_ref = stats.clone();
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
        .map_err(|e| DbError::InternalError(format!("Failed to create stats function: {}", e)))?;

    solidb
        .set("stats", stats_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set stats: {}", e)))?;

    Ok(())
}

fn setup_env_table_cached(
    engine: &ScriptEngine,
    lua: &Lua,
    solidb: &mlua::Table,
    db_name: &str,
    allow_secrets: bool,
) -> Result<(), DbError> {
    if !allow_secrets {
        let env_table = lua
            .create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create env table: {}", e)))?;
        solidb
            .set("env", env_table)
            .map_err(|e| DbError::InternalError(format!("Failed to set solidb.env: {}", e)))?;
        return Ok(());
    }
    let env_table = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create env table: {}", e)))?;

    // Check cache first
    if let Some(entry) = ENV_CACHE.get(db_name) {
        if entry.0.elapsed() < ENV_CACHE_TTL {
            // Use cached env
            for (key, value) in entry.1.iter() {
                env_table
                    .set(key.clone(), value.clone())
                    .map_err(|e| DbError::InternalError(format!("Failed to set env var: {}", e)))?;
            }
            solidb
                .set("env", env_table)
                .map_err(|e| DbError::InternalError(format!("Failed to set solidb.env: {}", e)))?;
            return Ok(());
        }
    }

    // Cache miss or expired: load from database
    let mut env_vars = std::collections::HashMap::new();

    if let Ok(db) = engine.storage.get_database(db_name) {
        if let Ok(collection) = db.system_collection("_env") {
            let collection: &crate::storage::Collection = &collection;
            let all_docs = collection.scan(None);
            for doc in all_docs {
                if let (Some(key), Some(value)) = (
                    doc.get("_key")
                        .and_then(|v| v.as_str().map(|s| s.to_string())),
                    doc.get("value")
                        .and_then(|v| v.as_str().map(|s| s.to_string())),
                ) {
                    env_vars.insert(key, value);
                }
            }
        }
    }

    // Populate env table
    for (key, value) in env_vars.iter() {
        env_table
            .set(key.clone(), value.clone())
            .map_err(|e| DbError::InternalError(format!("Failed to set env var: {}", e)))?;
    }

    // Update cache
    ENV_CACHE.insert(db_name.to_string(), (Instant::now(), env_vars));

    solidb
        .set("env", env_table)
        .map_err(|e| DbError::InternalError(format!("Failed to set solidb.env: {}", e)))?;

    Ok(())
}

fn setup_file_functions(
    lua: &Lua,
    solidb: &mlua::Table,
    storage: &Arc<StorageEngine>,
    db_name: &str,
) -> Result<(), DbError> {
    let upload_fn = create_upload_function(lua, storage.clone(), db_name.to_string())
        .map_err(|e| DbError::InternalError(format!("Failed to create upload function: {}", e)))?;
    solidb
        .set("upload", upload_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set upload: {}", e)))?;

    let file_info_fn = create_file_info_function(lua, storage.clone(), db_name.to_string())
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create file_info function: {}", e))
        })?;
    solidb
        .set("file_info", file_info_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set file_info: {}", e)))?;

    let file_read_fn = create_file_read_function(lua, storage.clone(), db_name.to_string())
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create file_read function: {}", e))
        })?;
    solidb
        .set("file_read", file_read_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set file_read: {}", e)))?;

    let file_delete_fn = create_file_delete_function(lua, storage.clone(), db_name.to_string())
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create file_delete function: {}", e))
        })?;
    solidb
        .set("file_delete", file_delete_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set file_delete: {}", e)))?;

    let file_list_fn = create_file_list_function(lua, storage.clone(), db_name.to_string())
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create file_list function: {}", e))
        })?;
    solidb
        .set("file_list", file_list_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set file_list: {}", e)))?;

    let image_process_fn = create_image_process_function(lua, storage.clone(), db_name.to_string())
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create image_process function: {}", e))
        })?;
    solidb
        .set("image_process", image_process_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set image_process: {}", e)))?;

    Ok(())
}

fn setup_streams_table(
    lua: &Lua,
    solidb: &mlua::Table,
    stream_manager: Arc<StreamManager>,
) -> Result<(), DbError> {
    let streams_table = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create streams table: {}", e)))?;

    // solidb.streams.list()
    let manager_list = stream_manager.clone();
    let list_fn = lua
        .create_function(move |lua, (): ()| {
            let streams = manager_list.list_streams();
            let mut result = Vec::new();
            for stream in streams {
                let mut s = serde_json::Map::new();
                s.insert("name".to_string(), serde_json::Value::String(stream.name));
                s.insert(
                    "created_at".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(stream.created_at)),
                );
                result.push(serde_json::Value::Object(s));
            }
            json_to_lua(lua, &serde_json::Value::Array(result))
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create streams.list: {}", e)))?;
    streams_table
        .set("list", list_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set streams.list: {}", e)))?;

    // solidb.streams.stop(name)
    let manager_stop = stream_manager.clone();
    let stop_fn = lua
        .create_function(move |_, name: String| {
            manager_stop
                .stop_stream(&name)
                .map_err(|e| mlua::Error::RuntimeError(e.to_string()))
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create streams.stop: {}", e)))?;
    streams_table
        .set("stop", stop_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set streams.stop: {}", e)))?;

    solidb
        .set("streams", streams_table)
        .map_err(|e| DbError::InternalError(format!("Failed to set solidb.streams: {}", e)))?;

    Ok(())
}

fn setup_db_object(engine: &ScriptEngine, lua: &Lua, db_name: &str) -> Result<(), DbError> {
    let globals = lua.globals();

    let db_handle = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create db table: {}", e)))?;
    db_handle
        .set("_name", db_name.to_string())
        .map_err(|e| DbError::InternalError(format!("Failed to set db name: {}", e)))?;

    // Setup all db methods (collection, query, transaction)
    setup_db_collection_method(lua, &db_handle, &engine.storage, db_name)?;
    setup_db_query_method(lua, &db_handle, &engine.storage, db_name)?;
    setup_db_transaction_method(lua, &db_handle, &engine.storage, db_name)?;

    globals
        .set("db", db_handle)
        .map_err(|e| DbError::InternalError(format!("Failed to set db global: {}", e)))?;

    Ok(())
}

fn setup_request_table(lua: &Lua, context: &ScriptContext) -> Result<(), DbError> {
    let globals = lua.globals();

    let request = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create request table: {}", e)))?;

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
    let headers = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create headers table: {}", e)))?;
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
        let body_lua = json_to_lua(lua, body)
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

    Ok(())
}

// Helper functions for db object methods (extracted for clarity)

fn setup_db_collection_method(
    lua: &Lua,
    db_handle: &mlua::Table,
    storage: &Arc<StorageEngine>,
    db_name: &str,
) -> Result<(), DbError> {
    let storage_ref = storage.clone();
    let current_db = db_name.to_string();

    let collection_fn = lua
        .create_function(move |lua, (_, coll_name): (LuaValue, String)| {
            let storage = storage_ref.clone();
            let db_name = current_db.clone();

            let coll_handle = lua.create_table()?;
            coll_handle.set("_solidb_handle", true)?;
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
            let insert_fn = lua.create_function(move |lua, (_, doc): (LuaValue, LuaValue)| {
                let json_doc = lua_to_json_value(lua, doc)?;

                let db = storage_insert
                    .get_database(&db_insert)
                    .map_err(mlua::Error::external)?;
                let collection = db
                    .get_collection_for_write(&coll_insert, caller_of(lua).actor)
                    .map_err(mlua::Error::external)?;

                let inserted = collection.insert(json_doc).map_err(mlua::Error::external)?;

                json_to_lua(lua, &inserted.to_value())
            })?;
            coll_handle.set("insert", insert_fn)?;

            // col:update(key, doc)
            let storage_update = storage.clone();
            let db_update = db_name.clone();
            let coll_update = coll_name.clone();
            let update_fn =
                lua.create_function(move |lua, (_, key, doc): (LuaValue, String, LuaValue)| {
                    let json_doc = lua_to_json_value(lua, doc)?;

                    let db = storage_update
                        .get_database(&db_update)
                        .map_err(mlua::Error::external)?;
                    let collection = db
                        .get_collection_for_write(&coll_update, caller_of(lua).actor)
                        .map_err(mlua::Error::external)?;

                    let updated = collection
                        .update(&key, json_doc)
                        .map_err(mlua::Error::external)?;

                    json_to_lua(lua, &updated.to_value())
                })?;
            coll_handle.set("update", update_fn)?;

            // col:delete(key)
            let storage_delete = storage.clone();
            let db_delete = db_name.clone();
            let coll_delete = coll_name.clone();
            let delete_fn = lua.create_function(move |lua, (_, key): (LuaValue, String)| {
                let db = storage_delete
                    .get_database(&db_delete)
                    .map_err(mlua::Error::external)?;
                let collection = db
                    .get_collection_for_write(&coll_delete, caller_of(lua).actor)
                    .map_err(mlua::Error::external)?;

                collection.delete(&key).map_err(mlua::Error::external)?;

                Ok(true)
            })?;
            coll_handle.set("delete", delete_fn)?;

            // col:count(filter?)
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

            // col:find(filter)
            let storage_find = storage.clone();
            let db_find = db_name.clone();
            let coll_find = coll_name.clone();
            let find_fn = lua.create_function(move |lua, (_, filter): (LuaValue, LuaValue)| {
                let filter_json = lua_to_json_value(lua, filter)?;

                let db = storage_find
                    .get_database(&db_find)
                    .map_err(mlua::Error::external)?;
                let collection = db
                    .get_collection(&coll_find)
                    .map_err(mlua::Error::external)?;

                let all_docs = collection.scan(None);
                let mut results = Vec::new();

                for doc in all_docs {
                    let doc_value = doc.to_value();
                    if matches_filter(&doc_value, &filter_json) {
                        results.push(doc_value);
                    }
                }

                let result_table = lua.create_table()?;
                for (i, doc) in results.iter().enumerate() {
                    result_table.set(i + 1, json_to_lua(lua, doc)?)?;
                }

                Ok(LuaValue::Table(result_table))
            })?;
            coll_handle.set("find", find_fn)?;

            // col:find_one(filter)
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

            // col:bulk_insert(docs)
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
                        .get_collection_for_write(&coll_bulk, caller_of(lua).actor)
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
                        let result = collection.insert(doc).map_err(mlua::Error::external)?;
                        inserted.push(result.to_value());
                    }

                    let result_table = lua.create_table()?;
                    for (i, doc) in inserted.iter().enumerate() {
                        result_table.set(i + 1, json_to_lua(lua, doc)?)?;
                    }

                    Ok(LuaValue::Table(result_table))
                })?;
            coll_handle.set("bulk_insert", bulk_insert_fn)?;

            // col:upsert(key_or_filter, doc)
            let storage_upsert = storage.clone();
            let db_upsert = db_name.clone();
            let coll_upsert = coll_name.clone();
            let upsert_fn = lua.create_function(
                move |lua, (_, key_or_filter, doc): (LuaValue, LuaValue, LuaValue)| {
                    let mut doc_json = lua_to_json_value(lua, doc)?;

                    let db = storage_upsert
                        .get_database(&db_upsert)
                        .map_err(mlua::Error::external)?;
                    // Checked before the lookup below: upsert reads first,
                    // but it is a write.
                    let collection = db
                        .get_collection_for_write(&coll_upsert, caller_of(lua).actor)
                        .map_err(mlua::Error::external)?;

                    let existing_key: Option<String> = match &key_or_filter {
                        LuaValue::String(s) => {
                            let key = s.to_str()?.to_string();
                            match collection.get(&key) {
                                Ok(_) => Some(key),
                                Err(_) => {
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
                        collection
                            .update(&key, doc_json)
                            .map_err(mlua::Error::external)?
                            .to_value()
                    } else {
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

    db_handle
        .set("collection", collection_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set collection function: {}", e)))?;

    Ok(())
}

fn setup_db_query_method(
    lua: &Lua,
    db_handle: &mlua::Table,
    storage: &Arc<StorageEngine>,
    db_name: &str,
) -> Result<(), DbError> {
    let storage_query = storage.clone();
    let db_query = db_name.to_string();
    let query_fn = lua
        .create_function(
            move |lua, (_, query, bind_vars): (LuaValue, String, Option<LuaValue>)| {
                let storage = storage_query.clone();

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

                let caller = caller_of(lua);
                let results = query_cached(
                    &storage,
                    &db_query,
                    &query,
                    bind_vars_map,
                    &caller.principal,
                )
                .map_err(mlua::Error::external)?;

                let result_table = lua.create_table()?;
                for (i, doc) in results.iter().enumerate() {
                    result_table.set(i + 1, json_to_lua(lua, doc)?)?;
                }

                Ok(LuaValue::Table(result_table))
            },
        )
        .map_err(|e| DbError::InternalError(format!("Failed to create query function: {}", e)))?;

    db_handle
        .set("query", query_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set query function: {}", e)))?;

    // db:query_json(sdbql, bind_vars?) -> returns raw JSON string, skipping Lua table conversion
    let storage_qj = storage.clone();
    let db_qj = db_name.to_string();
    let query_json_fn = lua
        .create_function(
            move |_lua, (_, query, bind_vars): (LuaValue, String, Option<LuaValue>)| {
                let storage = storage_qj.clone();

                let bind_vars_map = if let Some(vars) = bind_vars {
                    let json_vars = lua_to_json_value(_lua, vars)?;
                    if let JsonValue::Object(map) = json_vars {
                        map.into_iter().collect()
                    } else {
                        std::collections::HashMap::new()
                    }
                } else {
                    std::collections::HashMap::new()
                };

                let caller = caller_of(_lua);
                let json_str =
                    query_cached_json(&storage, &db_qj, &query, bind_vars_map, &caller.principal)
                        .map_err(mlua::Error::external)?;

                // Return as RawJson userdata — bypasses all conversion
                Ok(LuaValue::UserData(_lua.create_userdata(
                    crate::scripting::conversion::RawJson((*json_str).clone()),
                )?))
            },
        )
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create query_json function: {}", e))
        })?;

    db_handle
        .set("query_json", query_json_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set query_json: {}", e)))?;

    Ok(())
}

fn setup_db_transaction_method(
    lua: &Lua,
    db_handle: &mlua::Table,
    storage: &Arc<StorageEngine>,
    db_name: &str,
) -> Result<(), DbError> {
    let storage_tx = storage.clone();
    let db_tx = db_name.to_string();
    let transaction_fn = lua
        .create_async_function(move |lua, (_, callback): (LuaValue, mlua::Function)| {
            let storage = storage_tx.clone();
            let db_name = db_tx.clone();

            async move {
                storage
                    .initialize_transactions()
                    .map_err(mlua::Error::external)?;

                let tx_manager = storage
                    .transaction_manager()
                    .map_err(mlua::Error::external)?;

                let tx_id = tx_manager
                    .begin(crate::transaction::IsolationLevel::ReadCommitted)
                    .map_err(mlua::Error::external)?;

                let tx_handle = lua.create_table()?;
                tx_handle.set("_tx_id", tx_id.to_string())?;
                tx_handle.set("_db", db_name.clone())?;

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

                        let lock_manager = tx_manager.lock_manager().clone();

                        let coll_handle = lua.create_table()?;
                        coll_handle.set("_db", db_name.clone())?;
                        coll_handle.set("_name", coll_name.clone())?;
                        coll_handle.set("_tx_id", tx_id.to_string())?;

                        let storage_insert = storage.clone();
                        let tx_mgr_insert = tx_manager.clone();
                        let lock_mgr_insert = lock_manager.clone();
                        let db_insert = db_name.clone();
                        let coll_insert = coll_name.clone();
                        let tx_id_insert = tx_id;
                        let insert_fn =
                            lua.create_function(move |lua, (_, doc): (LuaValue, LuaValue)| {
                                let json_doc = lua_to_json_value(lua, doc)?;

                                let full_coll_name = format!("{}:{}", db_insert, coll_insert);
                                crate::storage::check_write_access(
                                    &full_coll_name,
                                    caller_of(lua).actor,
                                )
                                .map_err(mlua::Error::external)?;
                                let collection = storage_insert
                                    .get_collection(&full_coll_name)
                                    .map_err(mlua::Error::external)?;

                                let tx_arc = tx_mgr_insert
                                    .get(tx_id_insert)
                                    .map_err(mlua::Error::external)?;
                                let mut tx = tx_arc.write().unwrap();
                                let wal = tx_mgr_insert.wal().clone();
                                let lock_mgr = lock_mgr_insert.clone();

                                let inserted = collection
                                    .insert_tx(&mut tx, &wal, &lock_mgr, json_doc)
                                    .map_err(mlua::Error::external)?;

                                json_to_lua(lua, &inserted.to_value())
                            })?;
                        coll_handle.set("insert", insert_fn)?;

                        let storage_update = storage.clone();
                        let tx_mgr_update = tx_manager.clone();
                        let lock_mgr_update = lock_manager.clone();
                        let db_update = db_name.clone();
                        let coll_update = coll_name.clone();
                        let tx_id_update = tx_id;
                        let update_fn = lua.create_function(
                            move |lua, (_, key, doc): (LuaValue, String, LuaValue)| {
                                let json_doc = lua_to_json_value(lua, doc)?;

                                let full_coll_name = format!("{}:{}", db_update, coll_update);
                                crate::storage::check_write_access(
                                    &full_coll_name,
                                    caller_of(lua).actor,
                                )
                                .map_err(mlua::Error::external)?;
                                let collection = storage_update
                                    .get_collection(&full_coll_name)
                                    .map_err(mlua::Error::external)?;

                                let tx_arc = tx_mgr_update
                                    .get(tx_id_update)
                                    .map_err(mlua::Error::external)?;
                                let mut tx = tx_arc.write().unwrap();
                                let wal = tx_mgr_update.wal().clone();
                                let lock_mgr = lock_mgr_update.clone();

                                let updated = collection
                                    .update_tx(&mut tx, &wal, &lock_mgr, &key, json_doc)
                                    .map_err(mlua::Error::external)?;

                                json_to_lua(lua, &updated.to_value())
                            },
                        )?;
                        coll_handle.set("update", update_fn)?;

                        let storage_delete = storage.clone();
                        let tx_mgr_delete = tx_manager.clone();
                        let lock_mgr_delete = lock_manager.clone();
                        let db_delete = db_name.clone();
                        let coll_delete = coll_name.clone();
                        let tx_id_delete = tx_id;
                        let delete_fn =
                            lua.create_function(move |lua, (_, key): (LuaValue, String)| {
                                let full_coll_name = format!("{}:{}", db_delete, coll_delete);
                                crate::storage::check_write_access(
                                    &full_coll_name,
                                    caller_of(lua).actor,
                                )
                                .map_err(mlua::Error::external)?;
                                let collection = storage_delete
                                    .get_collection(&full_coll_name)
                                    .map_err(mlua::Error::external)?;

                                let tx_arc = tx_mgr_delete
                                    .get(tx_id_delete)
                                    .map_err(mlua::Error::external)?;
                                let mut tx = tx_arc.write().unwrap();
                                let wal = tx_mgr_delete.wal().clone();
                                let lock_mgr = lock_mgr_delete.clone();

                                collection
                                    .delete_tx(&mut tx, &wal, &lock_mgr, &key)
                                    .map_err(mlua::Error::external)?;

                                Ok(true)
                            })?;
                        coll_handle.set("delete", delete_fn)?;

                        let storage_get = storage.clone();
                        let lock_mgr_get = lock_manager.clone();
                        let db_get = db_name.clone();
                        let coll_get = coll_name.clone();
                        let tx_id_get = tx_id;
                        let get_fn =
                            lua.create_function(move |lua, (_, key): (LuaValue, String)| {
                                let full_coll_name = format!("{}:{}", db_get, coll_get);
                                let collection = storage_get
                                    .get_collection(&full_coll_name)
                                    .map_err(mlua::Error::external)?;

                                let lock_mgr = lock_mgr_get.clone();
                                match collection.get_tx(tx_id_get, &lock_mgr, &key) {
                                    Ok(Some(doc)) => json_to_lua(lua, &doc.to_value()),
                                    Ok(None) => Ok(LuaValue::Nil),
                                    Err(e) => Err(mlua::Error::external(e)),
                                }
                            })?;
                        coll_handle.set("get", get_fn)?;

                        Ok(LuaValue::Table(coll_handle))
                    })?;
                tx_handle.set("collection", tx_collection_fn)?;

                let result = callback
                    .call_async::<LuaValue>(LuaValue::Table(tx_handle))
                    .await;

                match result {
                    Ok(value) => {
                        storage
                            .commit_transaction(tx_id)
                            .map_err(mlua::Error::external)?;
                        Ok(value)
                    }
                    Err(e) => {
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

    Ok(())
}

pub fn setup_lua_globals(
    engine: &ScriptEngine,
    lua: &Lua,
    db_name: &str,
    context: &ScriptContext,
    script_info: Option<(&str, &str)>,
) -> Result<(), DbError> {
    let globals = lua.globals();
    lua.set_app_data(LuaCaller::from_context(context));
    crate::scripting::response::reset_overrides(lua);

    // Create 'solidb' namespace
    let solidb = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create solidb table: {}", e)))?;

    // solidb.log(msg)
    let storage_log = engine.storage.clone();
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

            let label = script_details
                .as_ref()
                .map(|(_, n)| n.as_str())
                .unwrap_or("Lua Script");
            tracing::info!("[{}] [{}] {}", db_log, label, msg);

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
    let stats_ref = engine.stats.clone();
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
        .map_err(|e| DbError::InternalError(format!("Failed to create stats function: {}", e)))?;
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

    // Setup string library extensions (regex, slugify, etc.)
    lua_globals::setup_string_extensions(lua)?;

    // Setup table library extensions (deep_merge, keys, values, etc.)
    lua_globals::setup_table_lib_extensions(lua)?;

    // solidb.fetch(url, options)
    let fetch_fn = lua_globals::create_fetch_function(lua)?;
    solidb
        .set("fetch", fetch_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set fetch: {}", e)))?;

    // Setup JSON globals (encode/decode)
    lua_globals::setup_json_globals(lua, &solidb)?;

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
        .map_err(|e| DbError::InternalError(format!("Failed to set validate_detailed: {}", e)))?;

    let sanitize_fn = create_sanitize_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create sanitize function: {}", e))
    })?;
    solidb
        .set("sanitize", sanitize_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set sanitize: {}", e)))?;

    let typeof_fn = create_typeof_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create typeof function: {}", e)))?;
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

    let cache_fn = create_cache_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create cache function: {}", e)))?;
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
    let error_fn = create_error_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create error function: {}", e)))?;
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
        .map_err(|e| DbError::InternalError(format!("Failed to set validate_condition: {}", e)))?;

    let check_permissions_fn = create_check_permissions_function(lua).map_err(|e| {
        DbError::InternalError(format!(
            "Failed to create check_permissions function: {}",
            e
        ))
    })?;
    solidb
        .set("check_permissions", check_permissions_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set check_permissions: {}", e)))?;

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

    let timeout_fn = create_timeout_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create timeout function: {}", e)))?;
    solidb
        .set("timeout", timeout_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set timeout: {}", e)))?;

    let retry_fn = create_retry_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create retry function: {}", e)))?;
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
    let upload_fn = create_upload_function(lua, engine.storage.clone(), db_name.to_string())
        .map_err(|e| DbError::InternalError(format!("Failed to create upload function: {}", e)))?;
    solidb
        .set("upload", upload_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set upload: {}", e)))?;

    let file_info_fn = create_file_info_function(lua, engine.storage.clone(), db_name.to_string())
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create file_info function: {}", e))
        })?;
    solidb
        .set("file_info", file_info_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set file_info: {}", e)))?;

    let file_read_fn = create_file_read_function(lua, engine.storage.clone(), db_name.to_string())
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create file_read function: {}", e))
        })?;
    solidb
        .set("file_read", file_read_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set file_read: {}", e)))?;

    let file_delete_fn =
        create_file_delete_function(lua, engine.storage.clone(), db_name.to_string()).map_err(
            |e| DbError::InternalError(format!("Failed to create file_delete function: {}", e)),
        )?;
    solidb
        .set("file_delete", file_delete_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set file_delete: {}", e)))?;

    let file_list_fn = create_file_list_function(lua, engine.storage.clone(), db_name.to_string())
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create file_list function: {}", e))
        })?;
    solidb
        .set("file_list", file_list_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set file_list: {}", e)))?;

    let image_process_fn =
        create_image_process_function(lua, engine.storage.clone(), db_name.to_string()).map_err(
            |e| DbError::InternalError(format!("Failed to create image_process function: {}", e)),
        )?;
    solidb
        .set("image_process", image_process_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set image_process: {}", e)))?;

    // Development Tools
    let debug_fn = create_debug_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create debug function: {}", e)))?;
    solidb
        .set("debug", debug_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set debug: {}", e)))?;

    let inspect_fn = create_inspect_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create inspect function: {}", e)))?;
    solidb
        .set("inspect", inspect_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set inspect: {}", e)))?;

    let profile_fn = create_profile_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create profile function: {}", e)))?;
    solidb
        .set("profile", profile_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set profile: {}", e)))?;

    let benchmark_fn = create_benchmark_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create benchmark function: {}", e))
    })?;
    solidb
        .set("benchmark", benchmark_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set benchmark: {}", e)))?;

    let mock_fn = create_mock_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create mock function: {}", e)))?;
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

    let dump_fn = create_dump_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create dump function: {}", e)))?;
    solidb
        .set("dump", dump_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set dump: {}", e)))?;

    // Set solidb global
    // Initialize solidb.env table
    let env_table = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create env table: {}", e)))?;

    // Secrets stay out of anonymous / non-admin scripts.
    if context.user.authenticated && context.user.has_role("admin") {
        if let Ok(db) = engine.storage.get_database(db_name) {
            if let Ok(collection) = db.system_collection("_env") {
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
    }

    // Create 'streams' module
    if let Some(stream_manager) = engine.stream_manager.clone() {
        let streams_table = lua.create_table().map_err(|e| {
            DbError::InternalError(format!("Failed to create streams table: {}", e))
        })?;

        // solidb.streams.list() -> array of {name: string, query: string, created_at: number}
        let manager_list = stream_manager.clone();
        let list_fn = lua
            .create_function(move |lua, (): ()| {
                let streams = manager_list.list_streams();
                let mut result = Vec::new();
                for stream in streams {
                    let mut s = serde_json::Map::new();
                    s.insert("name".to_string(), serde_json::Value::String(stream.name));
                    // We might not want to expose full complex query object, maybe just source collection?
                    // Or string representation if we had it.
                    // For now, let's just expose created_at
                    s.insert(
                        "created_at".to_string(),
                        serde_json::Value::Number(serde_json::Number::from(stream.created_at)),
                    );
                    result.push(serde_json::Value::Object(s));
                }

                // Use the json helper to convert to Lua table
                json_to_lua(lua, &serde_json::Value::Array(result))
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create streams.list: {}", e)))?;

        streams_table
            .set("list", list_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set streams.list: {}", e)))?;

        // solidb.streams.stop(name) -> void
        let manager_stop = stream_manager.clone();
        let stop_fn = lua
            .create_function(move |_, name: String| {
                manager_stop
                    .stop_stream(&name)
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))
            })
            .map_err(|e| DbError::InternalError(format!("Failed to create streams.stop: {}", e)))?;

        streams_table
            .set("stop", stop_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set streams.stop: {}", e)))?;

        solidb
            .set("streams", streams_table)
            .map_err(|e| DbError::InternalError(format!("Failed to set solidb.streams: {}", e)))?;
    }

    solidb
        .set("env", env_table)
        .map_err(|e| DbError::InternalError(format!("Failed to set solidb.env: {}", e)))?;

    // Add AI bindings (solidb.ai.*)
    let ai_table = ai_bindings::create_ai_table(lua, engine.storage.clone(), db_name)
        .map_err(|e| DbError::InternalError(format!("Failed to create AI table: {}", e)))?;
    solidb
        .set("ai", ai_table)
        .map_err(|e| DbError::InternalError(format!("Failed to set solidb.ai: {}", e)))?;

    globals
        .set("solidb", solidb)
        .map_err(|e| DbError::InternalError(format!("Failed to set solidb global: {}", e)))?;

    // Setup time globals (time.now, time.date, etc.)
    lua_globals::setup_time_globals(lua)?;

    // Setup table extensions (table.sorted, table.filter, etc.)
    lua_globals::setup_table_extensions(lua)?;

    // Global 'db' object: the same implementation as the pooled path, so
    // there is exactly one place that decides what a script may write.
    setup_db_object(engine, lua, db_name)?;

    // Create 'request' table with context info
    let request = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create request table: {}", e)))?;

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
    let headers = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create headers table: {}", e)))?;
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
        let body_lua = json_to_lua(lua, body)
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

    // Setup db object (query, query_json, collection, etc.)
    setup_db_object(engine, lua, db_name)?;

    // Create 'response' helper table
    // The `response` global: json / html / redirect / file / cors.
    let response = crate::scripting::response::create_response_table(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create response: {}", e)))?;

    globals
        .set("response", response)
        .map_err(|e| DbError::InternalError(format!("Failed to set response global: {}", e)))?;

    // Setup crypto namespace (md5, sha256, jwt, password hashing, etc.)
    lua_globals::setup_crypto_globals(lua)?;

    // Setup extended time namespace (now, sleep, format, parse, add, subtract)
    lua_globals::setup_time_ext_globals(lua)?;

    Ok(())
}
