//! Development Tools for Lua Scripts
//!
//! Provides debugging, profiling, and mocking utilities for script development.

use mlua::{Function, Lua, Result as LuaResult, Table, Value as LuaValue};
use std::time::Instant;

/// Create solidb.debug(data) -> string
/// Enhanced debugging that pretty-prints any value with type info
pub fn create_debug_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|_lua, value: LuaValue| {
        let output = format_value(&value, 0);

        // Log to tracing as well
        tracing::debug!(target: "lua_debug", "{}", output);

        // Return formatted string
        Ok(output)
    })
}

/// Create solidb.inspect(data) -> table
/// Returns detailed type information about a value
pub fn create_inspect_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|lua, value: LuaValue| {
        let result = lua.create_table()?;

        let type_name = match &value {
            LuaValue::Nil => "nil",
            LuaValue::Boolean(_) => "boolean",
            LuaValue::Integer(_) => "integer",
            LuaValue::Number(_) => "number",
            LuaValue::String(_) => "string",
            LuaValue::Table(_) => "table",
            LuaValue::Function(_) => "function",
            LuaValue::Thread(_) => "thread",
            LuaValue::UserData(_) => "userdata",
            LuaValue::LightUserData(_) => "lightuserdata",
            LuaValue::Error(_) => "error",
            _ => "unknown",
        };

        result.set("type", type_name)?;

        match &value {
            LuaValue::String(s) => {
                if let Ok(str_val) = s.to_str() {
                    result.set("length", str_val.len())?;
                    result.set("value", str_val)?;
                }
            }
            LuaValue::Integer(i) => {
                result.set("value", *i)?;
            }
            LuaValue::Number(n) => {
                result.set("value", *n)?;
            }
            LuaValue::Boolean(b) => {
                result.set("value", *b)?;
            }
            LuaValue::Table(t) => {
                let mut count = 0;
                let mut is_array = true;
                let mut max_index = 0i64;

                for pair in t.clone().pairs::<LuaValue, LuaValue>() {
                    if let Ok((k, _)) = pair {
                        count += 1;
                        match k {
                            LuaValue::Integer(i) => {
                                if i > max_index {
                                    max_index = i;
                                }
                            }
                            _ => is_array = false,
                        }
                    }
                }

                result.set("count", count)?;
                result.set("is_array", is_array && max_index == count as i64)?;

                // List keys
                let keys = lua.create_table()?;
                let mut idx = 1;
                for pair in t.clone().pairs::<LuaValue, LuaValue>() {
                    if let Ok((k, _)) = pair {
                        keys.set(idx, format_value(&k, 0))?;
                        idx += 1;
                    }
                }
                result.set("keys", keys)?;
            }
            _ => {}
        }

        Ok(result)
    })
}

/// Create solidb.profile(fn, args?) -> { result, duration_ms, memory_delta }
/// Profiles a function execution
pub fn create_profile_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|lua, (func, args): (Function, Option<LuaValue>)| {
        let start = Instant::now();

        // Execute the function
        let result: LuaValue = if let Some(a) = args {
            func.call(a)?
        } else {
            func.call(())?
        };

        let duration = start.elapsed();
        let duration_ms = duration.as_secs_f64() * 1000.0;
        let duration_us = duration.as_micros() as f64;

        // Build result table
        let profile_result = lua.create_table()?;
        profile_result.set("result", result)?;
        profile_result.set("duration_ms", duration_ms)?;
        profile_result.set("duration_us", duration_us)?;

        // Log profile info
        tracing::debug!(
            target: "lua_profile",
            duration_ms = duration_ms,
            "Function profiled"
        );

        Ok(profile_result)
    })
}

/// Create solidb.benchmark(fn, iterations?) -> { avg_ms, min_ms, max_ms, total_ms }
/// Runs a function multiple times and returns timing statistics
pub fn create_benchmark_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|lua, (func, iterations): (Function, Option<u32>)| {
        let iterations = iterations.unwrap_or(100);
        let mut times: Vec<f64> = Vec::with_capacity(iterations as usize);

        for _ in 0..iterations {
            let start = Instant::now();
            let _: LuaValue = func.call(())?;
            let duration = start.elapsed();
            times.push(duration.as_secs_f64() * 1000.0);
        }

        let total: f64 = times.iter().sum();
        let avg = total / iterations as f64;
        let min = times.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        // Calculate standard deviation
        let variance: f64 =
            times.iter().map(|t| (t - avg).powi(2)).sum::<f64>() / iterations as f64;
        let std_dev = variance.sqrt();

        let result = lua.create_table()?;
        result.set("iterations", iterations)?;
        result.set("total_ms", total)?;
        result.set("avg_ms", avg)?;
        result.set("min_ms", min)?;
        result.set("max_ms", max)?;
        result.set("std_dev_ms", std_dev)?;

        Ok(result)
    })
}

/// Create solidb.mock(name, data) -> mock table
/// Creates a mock collection-like object for testing
pub fn create_mock_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|lua, (name, initial_data): (String, Option<Table>)| {
        let mock = lua.create_table()?;
        let data = lua.create_table()?;

        // Copy initial data if provided
        if let Some(init) = initial_data {
            for pair in init.pairs::<LuaValue, LuaValue>() {
                if let Ok((k, v)) = pair {
                    data.set(k, v)?;
                }
            }
        }

        mock.set("_name", name)?;
        mock.set("_data", data.clone())?;
        mock.set("_call_log", lua.create_table()?)?;

        // mock:get(key)
        let data_get = data.clone();
        let get_fn = lua.create_function(move |_lua, (_self, key): (Table, String)| {
            let val: LuaValue = data_get.get(key.clone())?;
            Ok(val)
        })?;
        mock.set("get", get_fn)?;

        // mock:insert(doc)
        let data_insert = data.clone();
        let insert_fn = lua.create_function(move |lua, (_self, doc): (Table, Table)| {
            let key: Option<String> = doc.get("_key").ok();
            let actual_key = key.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            // Clone doc and set _key
            let new_doc = lua.create_table()?;
            new_doc.set("_key", actual_key.clone())?;
            for pair in doc.pairs::<String, LuaValue>() {
                if let Ok((k, v)) = pair {
                    if k != "_key" {
                        new_doc.set(k, v)?;
                    }
                }
            }

            data_insert.set(actual_key.clone(), new_doc.clone())?;
            Ok(new_doc)
        })?;
        mock.set("insert", insert_fn)?;

        // mock:update(key, doc)
        let data_update = data.clone();
        let update_fn =
            lua.create_function(move |_lua, (_self, key, updates): (Table, String, Table)| {
                let existing: Option<Table> = data_update.get(key.clone()).ok();
                if let Some(doc) = existing {
                    for pair in updates.pairs::<String, LuaValue>() {
                        if let Ok((k, v)) = pair {
                            doc.set(k, v)?;
                        }
                    }
                    Ok(doc)
                } else {
                    Err(mlua::Error::RuntimeError(format!(
                        "Document not found: {}",
                        key
                    )))
                }
            })?;
        mock.set("update", update_fn)?;

        // mock:delete(key)
        let data_delete = data.clone();
        let delete_fn = lua.create_function(move |_lua, (_self, key): (Table, String)| {
            let existed: bool = data_delete.contains_key(key.clone())?;
            if existed {
                data_delete.set(key, LuaValue::Nil)?;
            }
            Ok(existed)
        })?;
        mock.set("delete", delete_fn)?;

        // mock:all()
        let data_all = data.clone();
        let all_fn = lua.create_function(move |lua, _self: Table| {
            let result = lua.create_table()?;
            let mut idx = 1;
            for pair in data_all.clone().pairs::<String, Table>() {
                if let Ok((_, v)) = pair {
                    result.set(idx, v)?;
                    idx += 1;
                }
            }
            Ok(result)
        })?;
        mock.set("all", all_fn)?;

        // mock:count()
        let data_count = data.clone();
        let count_fn = lua.create_function(move |_lua, _self: Table| {
            let mut count = 0;
            for _ in data_count.clone().pairs::<LuaValue, LuaValue>() {
                count += 1;
            }
            Ok(count)
        })?;
        mock.set("count", count_fn)?;

        // mock:reset()
        let data_reset = data.clone();
        let reset_fn = lua.create_function(move |_lua, _self: Table| {
            // Clear all data
            let keys: Vec<String> = data_reset
                .clone()
                .pairs::<String, LuaValue>()
                .filter_map(|r| r.ok().map(|(k, _)| k))
                .collect();
            for key in keys {
                data_reset.set(key, LuaValue::Nil)?;
            }
            Ok(())
        })?;
        mock.set("reset", reset_fn)?;

        Ok(mock)
    })
}

/// Create solidb.assert(condition, message?) - assertion helper
pub fn create_dev_assert_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|_lua, (condition, message): (bool, Option<String>)| {
        if !condition {
            let msg = message.unwrap_or_else(|| "Assertion failed".to_string());
            return Err(mlua::Error::RuntimeError(msg));
        }
        Ok(true)
    })
}

/// Create solidb.assert_eq(a, b, message?) - equality assertion
pub fn create_assert_eq_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(
        |_lua, (a, b, message): (LuaValue, LuaValue, Option<String>)| {
            let equal = values_equal(&a, &b);
            if !equal {
                let msg = message.unwrap_or_else(|| {
                    format!(
                        "Assertion failed: {} != {}",
                        format_value(&a, 0),
                        format_value(&b, 0)
                    )
                });
                return Err(mlua::Error::RuntimeError(msg));
            }
            Ok(true)
        },
    )
}

/// Create solidb.dump(value) -> string (JSON-like dump)
pub fn create_dump_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|_lua, value: LuaValue| Ok(format_value(&value, 0)))
}

/// Format a Lua value for display
fn format_value(value: &LuaValue, indent: usize) -> String {
    let spaces = "  ".repeat(indent);
    match value {
        LuaValue::Nil => "nil".to_string(),
        LuaValue::Boolean(b) => b.to_string(),
        LuaValue::Integer(i) => i.to_string(),
        LuaValue::Number(n) => format!("{}", n),
        LuaValue::String(s) => {
            if let Ok(str_val) = s.to_str() {
                format!("\"{}\"", str_val.escape_default())
            } else {
                "\"<invalid utf8>\"".to_string()
            }
        }
        LuaValue::Table(t) => {
            let mut parts = Vec::new();
            let mut is_array = true;
            let mut expected_idx = 1i64;

            // First pass: check if array
            for pair in t.clone().pairs::<LuaValue, LuaValue>() {
                if let Ok((k, _)) = pair {
                    match k {
                        LuaValue::Integer(i) if i == expected_idx => {
                            expected_idx += 1;
                        }
                        _ => {
                            is_array = false;
                            break;
                        }
                    }
                }
            }

            // Second pass: format
            if is_array {
                for pair in t.clone().pairs::<i64, LuaValue>() {
                    if let Ok((_, v)) = pair {
                        parts.push(format_value(&v, indent + 1));
                    }
                }
                if parts.is_empty() {
                    "[]".to_string()
                } else if parts.len() <= 3 && parts.iter().all(|p| !p.contains('\n')) {
                    format!("[{}]", parts.join(", "))
                } else {
                    let inner_spaces = "  ".repeat(indent + 1);
                    format!(
                        "[\n{}{}\n{}]",
                        inner_spaces,
                        parts.join(&format!(",\n{}", inner_spaces)),
                        spaces
                    )
                }
            } else {
                for pair in t.clone().pairs::<LuaValue, LuaValue>() {
                    if let Ok((k, v)) = pair {
                        let key_str = match &k {
                            LuaValue::String(s) => s
                                .to_str()
                                .map(|s| s.to_string())
                                .unwrap_or_else(|_| "?".to_string()),
                            LuaValue::Integer(i) => format!("[{}]", i),
                            _ => format!("[{}]", format_value(&k, 0)),
                        };
                        parts.push(format!("{}: {}", key_str, format_value(&v, indent + 1)));
                    }
                }
                if parts.is_empty() {
                    "{}".to_string()
                } else if parts.len() <= 2
                    && parts.iter().all(|p| !p.contains('\n') && p.len() < 40)
                {
                    format!("{{ {} }}", parts.join(", "))
                } else {
                    let inner_spaces = "  ".repeat(indent + 1);
                    format!(
                        "{{\n{}{}\n{}}}",
                        inner_spaces,
                        parts.join(&format!(",\n{}", inner_spaces)),
                        spaces
                    )
                }
            }
        }
        LuaValue::Function(_) => "<function>".to_string(),
        LuaValue::Thread(_) => "<thread>".to_string(),
        LuaValue::UserData(_) => "<userdata>".to_string(),
        LuaValue::LightUserData(_) => "<lightuserdata>".to_string(),
        LuaValue::Error(e) => format!("<error: {}>", e),
        _ => "<unknown>".to_string(),
    }
}

/// Compare two Lua values for equality
fn values_equal(a: &LuaValue, b: &LuaValue) -> bool {
    match (a, b) {
        (LuaValue::Nil, LuaValue::Nil) => true,
        (LuaValue::Boolean(a), LuaValue::Boolean(b)) => a == b,
        (LuaValue::Integer(a), LuaValue::Integer(b)) => a == b,
        (LuaValue::Number(a), LuaValue::Number(b)) => (a - b).abs() < f64::EPSILON,
        (LuaValue::Integer(a), LuaValue::Number(b)) => (*a as f64 - b).abs() < f64::EPSILON,
        (LuaValue::Number(a), LuaValue::Integer(b)) => (a - *b as f64).abs() < f64::EPSILON,
        (LuaValue::String(a), LuaValue::String(b)) => a.as_bytes() == b.as_bytes(),
        (LuaValue::Table(a), LuaValue::Table(b)) => {
            // Simple table comparison
            let mut a_count = 0;
            let mut b_count = 0;

            for pair in a.clone().pairs::<LuaValue, LuaValue>() {
                if let Ok((k, v)) = pair {
                    a_count += 1;
                    if let Ok(bv) = b.get::<LuaValue>(k) {
                        if !values_equal(&v, &bv) {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
            }

            for _ in b.clone().pairs::<LuaValue, LuaValue>() {
                b_count += 1;
            }

            a_count == b_count
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_value_primitives() {
        assert_eq!(format_value(&LuaValue::Nil, 0), "nil");
        assert_eq!(format_value(&LuaValue::Boolean(true), 0), "true");
        assert_eq!(format_value(&LuaValue::Integer(42), 0), "42");
    }

    #[test]
    fn test_values_equal() {
        assert!(values_equal(&LuaValue::Nil, &LuaValue::Nil));
        assert!(values_equal(
            &LuaValue::Boolean(true),
            &LuaValue::Boolean(true)
        ));
        assert!(!values_equal(
            &LuaValue::Boolean(true),
            &LuaValue::Boolean(false)
        ));
        assert!(values_equal(&LuaValue::Integer(42), &LuaValue::Integer(42)));
        assert!(values_equal(
            &LuaValue::Integer(42),
            &LuaValue::Number(42.0)
        ));
    }
}
