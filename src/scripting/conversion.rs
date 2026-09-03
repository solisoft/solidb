use mlua::{Lua, Result as LuaResult, UserData, Value as LuaValue};
use serde_json::Value as JsonValue;

/// Wrapper for pre-serialized JSON strings that bypass Lua table conversion.
/// Used by `db:query_json()` to avoid the JSON→Lua→JSON roundtrip.
#[derive(Clone)]
pub struct RawJson(pub String);

impl UserData for RawJson {}

/// Convert JSON value to Lua value
pub fn json_to_lua(lua: &Lua, json: &JsonValue) -> LuaResult<LuaValue> {
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

/// Convert Lua value to JSON value (by reference)
pub fn lua_value_to_json(value: &LuaValue) -> LuaResult<JsonValue> {
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
pub fn matches_filter(doc: &JsonValue, filter: &JsonValue) -> bool {
    match filter {
        JsonValue::Object(filter_obj) => {
            for (key, filter_value) in filter_obj {
                match doc.get(key) {
                    Some(doc_value) => {
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

/// Deepest table nesting converted to JSON. serde_json refuses to parse
/// deeper than 128 itself, so nothing is lost.
const MAX_LUA_JSON_DEPTH: usize = 128;

/// Convert Lua value to JSON value (by value)
pub fn lua_to_json_value(lua: &Lua, value: LuaValue) -> LuaResult<JsonValue> {
    lua_to_json_value_at(lua, value, 0)
}

/// The recursion behind `lua_to_json_value`.
///
/// A script can hand over a cyclic table (`t[1] = t`) or one nested a
/// million deep; both recursed on the Rust stack until the process
/// segfaulted, out of reach of the Lua instruction hook and memory limit.
#[allow(clippy::only_used_in_recursion)]
fn lua_to_json_value_at(lua: &Lua, value: LuaValue, depth: usize) -> LuaResult<JsonValue> {
    if depth > MAX_LUA_JSON_DEPTH {
        return Err(mlua::Error::external(format!(
            "table nesting too deep (max {}); is it cyclic?",
            MAX_LUA_JSON_DEPTH
        )));
    }
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
            let mut entries = 0usize;

            for pair in t.clone().pairs::<LuaValue, LuaValue>() {
                let (k, _) = pair?;
                entries += 1;
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

            // A sparse table is an array only while the holes are cheap:
            // `{[1000000000] = 1}` used to size a Vec by its largest key
            // (the same ratio rule as lua-cjson's `encode_sparse_array`).
            // `with_capacity` on a key like that is an allocation failure,
            // which aborts rather than panics.
            let dense_enough = max_index <= 10 || (max_index as usize) <= entries.saturating_mul(2);
            if is_array && max_index > 0 && dense_enough {
                let mut arr = Vec::with_capacity(max_index as usize);
                for i in 1..=max_index {
                    let v: LuaValue = t.get(i)?;
                    arr.push(lua_to_json_value_at(lua, v, depth + 1)?);
                }
                Ok(JsonValue::Array(arr))
            } else {
                let mut obj = serde_json::Map::new();
                for pair in t.pairs::<String, LuaValue>() {
                    let (k, v) = pair?;
                    obj.insert(k, lua_to_json_value_at(lua, v, depth + 1)?);
                }
                Ok(JsonValue::Object(obj))
            }
        }
        LuaValue::UserData(ud) => {
            // Check for RawJson userdata — pre-serialized JSON string
            if let Ok(raw) = ud.borrow::<RawJson>() {
                return serde_json::from_str(&raw.0)
                    .map_err(|e| mlua::Error::external(format!("Invalid RawJson: {}", e)));
            }
            Ok(JsonValue::Null)
        }
        _ => Ok(JsonValue::Null), // Functions, etc. become null
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

#[cfg(test)]
mod bounds_tests {
    use super::*;

    /// `t[1] = t` recursed on the Rust stack until the process segfaulted;
    /// nothing in the Lua sandbox could see it.
    #[test]
    fn cyclic_table_is_an_error() {
        let lua = Lua::new();
        let t: LuaValue = lua.load("local t = {} t[1] = t return t").eval().unwrap();
        let err = lua_to_json_value(&lua, t).unwrap_err();
        assert!(err.to_string().contains("nesting too deep"), "{err}");
    }

    #[test]
    fn deeply_nested_table_is_an_error() {
        let lua = Lua::new();
        let t: LuaValue = lua
            .load("local t = {} for _ = 1, 100000 do t = { t } end return t")
            .eval()
            .unwrap();
        assert!(lua_to_json_value(&lua, t).is_err());
    }

    /// `{[1000000000] = 1}` used to size a Vec by its largest key.
    #[test]
    fn very_sparse_table_becomes_an_object() {
        let lua = Lua::new();
        let t: LuaValue = lua.load("return {[1000000000] = 1}").eval().unwrap();
        let json = lua_to_json_value(&lua, t).unwrap();
        assert_eq!(json, serde_json::json!({"1000000000": 1}));

        // Small holes are still an array, as before.
        let t: LuaValue = lua.load("return {1, nil, 3}").eval().unwrap();
        let json = lua_to_json_value(&lua, t).unwrap();
        assert_eq!(json, serde_json::json!([1, null, 3]));
    }
}
