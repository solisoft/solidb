//! JSON encode/decode functions for Lua

use crate::error::DbError;
use crate::scripting::conversion::{json_to_lua, lua_to_json_value};
use mlua::{Lua, Value as LuaValue};
use serde_json::Value as JsonValue;

/// Setup JSON globals as static (without solidb table reference)
/// This creates the global `json` table with encode/decode functions.
/// Used for static initialization in pool states.
pub fn setup_json_globals_static(lua: &Lua) -> Result<(), DbError> {
    let globals = lua.globals();

    // json.encode(value) -> string
    let json_encode_fn = lua
        .create_function(|lua, val: LuaValue| {
            let json_val = lua_to_json_value(lua, val)?;
            serde_json::to_string(&json_val).map_err(mlua::Error::external)
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create json_encode function: {}", e))
        })?;

    // json.decode(string) -> value
    let json_decode_fn = lua
        .create_function(|lua, s: String| {
            let json_val: JsonValue = serde_json::from_str(&s).map_err(mlua::Error::external)?;
            json_to_lua(lua, &json_val)
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create json_decode function: {}", e))
        })?;

    // Create global json table
    let json_table = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create json table: {}", e)))?;
    json_table
        .set("encode", json_encode_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set json.encode: {}", e)))?;
    json_table
        .set("decode", json_decode_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set json.decode: {}", e)))?;
    globals
        .set("json", json_table)
        .map_err(|e| DbError::InternalError(format!("Failed to set global json: {}", e)))?;

    Ok(())
}

/// Setup JSON globals (solidb.json_encode, solidb.json_decode, and global json table)
pub fn setup_json_globals(lua: &Lua, solidb: &mlua::Table) -> Result<(), DbError> {
    let globals = lua.globals();

    // solidb.json_encode(value) -> string
    let json_encode_fn = lua
        .create_function(|lua, val: LuaValue| {
            let json_val = lua_to_json_value(lua, val)?;
            serde_json::to_string(&json_val).map_err(mlua::Error::external)
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create json_encode function: {}", e))
        })?;
    solidb
        .set("json_encode", json_encode_fn.clone())
        .map_err(|e| DbError::InternalError(format!("Failed to set json_encode: {}", e)))?;

    // solidb.json_decode(string) -> value
    let json_decode_fn = lua
        .create_function(|lua, s: String| {
            let json_val: JsonValue = serde_json::from_str(&s).map_err(mlua::Error::external)?;
            json_to_lua(lua, &json_val)
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create json_decode function: {}", e))
        })?;
    solidb
        .set("json_decode", json_decode_fn.clone())
        .map_err(|e| DbError::InternalError(format!("Failed to set json_decode: {}", e)))?;

    // Create global json table for convenience (json.encode / json.decode)
    let json_table = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create json table: {}", e)))?;
    json_table
        .set("encode", json_encode_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set json.encode: {}", e)))?;
    json_table
        .set("decode", json_decode_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set json.decode: {}", e)))?;
    globals
        .set("json", json_table)
        .map_err(|e| DbError::InternalError(format!("Failed to set global json: {}", e)))?;

    Ok(())
}
