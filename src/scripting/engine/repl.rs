use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use mlua::{Lua, Value as LuaValue};
use serde_json::Value as JsonValue;

use crate::error::DbError;
use crate::scripting::conversion::json_to_lua;

use super::ScriptEngine;

#[allow(clippy::too_many_arguments)]
pub async fn execute_repl(
    engine: &ScriptEngine,
    code: &str,
    db_name: &str,
    user: crate::scripting::auth::ScriptUser,
    variables: &HashMap<String, JsonValue>,
    history: &[String],
    output_capture: &mut Vec<String>,
    timeout_ms: u64,
) -> Result<(JsonValue, HashMap<String, JsonValue>), DbError> {
    engine.stats.active_scripts.fetch_add(1, Ordering::SeqCst);
    engine
        .stats
        .total_scripts_executed
        .fetch_add(1, Ordering::SeqCst);

    // Ensure active counter is decremented even on panic or early return
    struct ActiveScriptGuard(Arc<crate::scripting::types::ScriptStats>);
    impl Drop for ActiveScriptGuard {
        fn drop(&mut self) {
            self.0.active_scripts.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let _guard = ActiveScriptGuard(engine.stats.clone());

    let lua = Lua::new();
    // Audit A4: the REPL had neither the allocator cap nor the deadline the
    // other script paths have, so `while true do end` pinned a tokio worker
    // for good and a growing table could take the host down. The budget is
    // the caller's `timeout_ms`, never more than the script timeout.
    super::pool::LuaPool::apply_memory_limit(&lua);
    let limit = repl_time_limit(timeout_ms);
    if let Some(limit) = limit {
        super::install_deadline_hook_for(&lua, limit);
    }

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
    let context = crate::scripting::types::ScriptContext {
        method: "REPL".to_string(),
        path: "repl".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
        is_websocket: false,
        // The REPL caller, not an anonymous one: the endpoint is permission
        // gated, and what the session may write follows from who opened it.
        user,
    };

    // Set up the Lua environment (script info is None for REPL)
    engine.setup_lua_globals(&lua, db_name, &context, None)?;

    // Inject session variables into global scope
    for (name, value) in variables {
        // Check if this is a saved collection handle that needs recreation
        if let JsonValue::Object(ref obj) = value {
            if obj.get("_solidb_handle").and_then(|v| v.as_bool()) == Some(true) {
                // Recreate collection handle using db:collection(), called
                // from Rust: splicing the names into Lua source let a
                // crafted global name or collection name inject code.
                if let Some(coll_name) = obj.get("_name").and_then(|v| v.as_str()) {
                    if let Ok(handle) = recreate_collection_handle(&lua, coll_name) {
                        let _ = globals.set(name.clone(), handle);
                    }
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
    replay_function_definitions(&lua, history);

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
                    if let Ok(json) = table_to_json_static(lua, t) {
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
        .map_err(|e| DbError::InternalError(format!("Failed to create capture log fn: {}", e)))?;

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
                        if let Ok(json) = table_to_json_static(lua, t) {
                            serde_json::to_string(&json).unwrap_or_else(|_| "[table]".to_string())
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

    let result = match super::eval_with_deadline(chunk, limit).await {
        Ok(result) => {
            // Convert Lua result to JSON
            let json_result = engine.lua_to_json(&lua, result)?;
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
        "solidb",
        "string",
        "table",
        "math",
        "utf8",
        "bit32",
        "coroutine",
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
        "_G",
        "_VERSION",
        "db",
        "request",
        "response",
        "time",
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
            if let Ok(json_val) = engine.lua_to_json(&lua, val) {
                updated_vars.insert(name, json_val);
            }
        }
    }

    super::remove_deadline_hook(&lua);

    match result {
        Ok(json_result) => Ok((json_result, updated_vars)),
        Err(e) => Err(e),
    }
}

/// The REPL's wall-clock budget: the requested `timeout_ms`, capped by the
/// script timeout. `timeout_ms == 0` means "the script timeout".
fn repl_time_limit(timeout_ms: u64) -> Option<std::time::Duration> {
    let requested = (timeout_ms > 0).then(|| std::time::Duration::from_millis(timeout_ms));
    match (requested, super::script_timeout()) {
        (Some(r), Some(cap)) => Some(r.min(cap)),
        (Some(r), None) => Some(r),
        (None, cap) => cap,
    }
}

fn recreate_collection_handle(lua: &Lua, coll_name: &str) -> mlua::Result<LuaValue> {
    let db: mlua::Table = lua.globals().get("db")?;
    let collection: mlua::Function = db.get("collection")?;
    collection.call((db, coll_name))
}

/// Globals a replayed history entry may read. Pure functions only: nothing
/// that reaches the database, the network, or the session output.
const REPLAY_SAFE_GLOBALS: &[&str] = &[
    "assert",
    "error",
    "getmetatable",
    "ipairs",
    "math",
    "next",
    "pairs",
    "pcall",
    "rawequal",
    "rawget",
    "rawlen",
    "rawset",
    "select",
    "setmetatable",
    "string",
    "table",
    "tonumber",
    "tostring",
    "type",
    "utf8",
    "xpcall",
];

/// Re-create the functions an earlier REPL command defined.
///
/// Functions cannot be stored as JSON, so earlier commands that define one
/// are run again on every eval. They used to be re-executed *whole* in the
/// real environment, so `f = function() end; db:collection("x"):insert(...)`
/// repeated its insert on every later eval (audit A4).
///
/// Each such command now runs in a scratch environment that holds only
/// [`REPLAY_SAFE_GLOBALS`]: no `db`, no `solidb`, no `print`, so its top
/// level can compute but not act — a statement that reaches for the
/// database fails there, harmlessly. Only the functions it assigned are
/// kept. The scratch environment is then emptied and made to read and write
/// through to the real globals, so when a replayed function is later
/// *called* it sees the session's `db`, variables and other functions as
/// before. The replay runs under the same deadline and memory cap as the
/// command itself.
fn replay_function_definitions(lua: &Lua, history: &[String]) {
    let candidates: Vec<&String> = history
        .iter()
        .filter(|code| code.contains("function"))
        .collect();
    if candidates.is_empty() {
        return;
    }
    let globals = lua.globals();
    let Ok(env) = lua.create_table() else {
        return;
    };
    for name in REPLAY_SAFE_GLOBALS {
        if let Ok(v) = globals.get::<LuaValue>(*name) {
            let _ = env.raw_set(*name, v);
        }
    }
    for code in candidates {
        let _ = lua.load(code.as_str()).set_environment(env.clone()).exec();
    }

    let defined: Vec<(LuaValue, mlua::Function)> = env
        .pairs::<LuaValue, LuaValue>()
        .filter_map(|r| r.ok())
        .filter_map(|(k, v)| {
            let LuaValue::Function(f) = v else {
                return None;
            };
            let keep = match &k {
                LuaValue::String(s) => {
                    let name = s.to_string_lossy();
                    !REPLAY_SAFE_GLOBALS.iter().any(|g| *g == &*name)
                }
                _ => false,
            };
            keep.then_some((k, f))
        })
        .collect();

    // Replayed functions resolve globals through `env`; from here on it is
    // a window onto the real globals, not a stale copy.
    let _ = env.clear();
    if let Ok(mt) = lua.create_table() {
        let _ = mt.raw_set("__index", globals.clone());
        let _ = mt.raw_set("__newindex", globals.clone());
        let _ = env.set_metatable(Some(mt));
    }
    for (name, f) in defined {
        let _ = globals.set(name, f);
    }
}

/// Static helper for table_to_json used in closures
pub(crate) fn table_to_json_static(
    lua: &Lua,
    table: mlua::Table,
) -> Result<JsonValue, mlua::Error> {
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
            arr.push(lua_value_to_json_static(lua, val)?);
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
            map.insert(key_str, lua_value_to_json_static(lua, v)?);
        }
        Ok(JsonValue::Object(map))
    }
}

/// Static helper for lua_value to json conversion
pub(crate) fn lua_value_to_json_static(
    lua: &Lua,
    value: LuaValue,
) -> Result<JsonValue, mlua::Error> {
    match value {
        LuaValue::Nil => Ok(JsonValue::Null),
        LuaValue::Boolean(b) => Ok(JsonValue::Bool(b)),
        LuaValue::Integer(i) => Ok(JsonValue::Number(i.into())),
        LuaValue::Number(n) => Ok(serde_json::Number::from_f64(n)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null)),
        LuaValue::String(s) => Ok(JsonValue::String(s.to_str()?.to_string())),
        LuaValue::Table(t) => table_to_json_static(lua, t),
        _ => Ok(JsonValue::Null),
    }
}
