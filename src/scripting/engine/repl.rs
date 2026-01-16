use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use mlua::{Lua, Value as LuaValue};
use serde_json::Value as JsonValue;

use crate::error::DbError;
use crate::scripting::conversion::json_to_lua;

use super::ScriptEngine;

pub async fn execute_repl(
    engine: &ScriptEngine,
    code: &str,
    db_name: &str,
    variables: &HashMap<String, JsonValue>,
    history: &[String],
    output_capture: &mut Vec<String>,
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
        user: crate::scripting::auth::ScriptUser::anonymous(),
    };

    // Set up the Lua environment (script info is None for REPL)
    engine.setup_lua_globals(&lua, db_name, &context, None)?;

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

    let result = match chunk.eval_async::<LuaValue>().await {
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

    match result {
        Ok(json_result) => Ok((json_result, updated_vars)),
        Err(e) => Err(e),
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
