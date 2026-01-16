//! Table library extensions for Lua (Rust-based)

use crate::error::DbError;
use crate::scripting::string_utils::*;
use mlua::Lua;

/// Setup table library extensions (deep_merge, keys, values, etc.)
pub fn setup_table_lib_extensions(lua: &Lua) -> Result<(), DbError> {
    let globals = lua.globals();

    let table_lib: mlua::Table = globals
        .get("table")
        .map_err(|e| DbError::InternalError(format!("Failed to get table library: {}", e)))?;

    // table.deep_merge(t1, t2) - Recursive table merging
    let deep_merge_fn = create_deep_merge_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create deep_merge function: {}", e))
    })?;
    table_lib
        .set("deep_merge", deep_merge_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set table.deep_merge: {}", e)))?;

    // table.keys(t) - Get array of keys
    let keys_fn = create_keys_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create keys function: {}", e)))?;
    table_lib
        .set("keys", keys_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set table.keys: {}", e)))?;

    // table.values(t) - Get array of values
    let values_fn = create_values_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create values function: {}", e)))?;
    table_lib
        .set("values", values_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set table.values: {}", e)))?;

    // table.contains(t, value) - Check if table contains value
    let contains_fn = create_contains_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create contains function: {}", e))
    })?;
    table_lib
        .set("contains", contains_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set table.contains: {}", e)))?;

    // table.filter(t, predicate) - Filter table by predicate function
    let filter_fn = create_filter_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create filter function: {}", e)))?;
    table_lib
        .set("filter", filter_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set table.filter: {}", e)))?;

    // table.map(t, transform) - Transform table values
    let map_fn = create_map_function(lua)
        .map_err(|e| DbError::InternalError(format!("Failed to create map function: {}", e)))?;
    table_lib
        .set("map", map_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set table.map: {}", e)))?;

    Ok(())
}
