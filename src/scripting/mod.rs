//! Lua Scripting Engine for Custom API Endpoints
//!
//! This module provides embedded Lua scripting capabilities that allow users
//! to create custom API endpoints with full access to database operations.

use mlua::{Lua, Result as LuaResult, Value as LuaValue};
use serde_json::Value as JsonValue;
use std::sync::Arc;

use crate::error::DbError;
use crate::storage::StorageEngine;
use crate::sdbql::{parse, QueryExecutor};

/// Context passed to Lua scripts containing request information
#[derive(Debug, Clone)]
pub struct ScriptContext {
    /// HTTP method (GET, POST, PUT, DELETE)
    pub method: String,
    /// Request path (after /api/custom/)
    pub path: String,
    /// Query parameters
    pub query_params: std::collections::HashMap<String, String>,
    /// URL parameters (e.g., :id)
    pub params: std::collections::HashMap<String, String>,
    /// Request headers
    pub headers: std::collections::HashMap<String, String>,
    /// Request body (parsed as JSON if applicable)
    pub body: Option<JsonValue>,
}

/// Script metadata stored in _system/_scripts
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Script {
    #[serde(rename = "_key")]
    pub key: String,
    /// Human-readable name
    pub name: String,
    /// HTTP methods this script handles (e.g., ["GET", "POST"])
    pub methods: Vec<String>,
    /// URL path pattern (e.g., "users/:id" or "hello")
    pub path: String,
    /// Database this script belongs to
    #[serde(default = "default_database")]
    pub database: String,
    /// Collection this script is scoped to (optional)
    pub collection: Option<String>,
    /// The Lua source code
    pub code: String,
    /// Optional description
    pub description: Option<String>,
    /// Creation timestamp
    pub created_at: String,
    /// Last modified timestamp
    /// Last modified timestamp
    pub updated_at: String,
}

fn default_database() -> String {
    "_system".to_string()
}

/// Lua scripting engine
pub struct ScriptEngine {
    storage: Arc<StorageEngine>,
}

impl ScriptEngine {
    /// Create a new script engine with access to the storage layer
    pub fn new(storage: Arc<StorageEngine>) -> Self {
        Self { storage }
    }

    /// Execute a Lua script with the given context
    pub fn execute(&self, script: &Script, db_name: &str, context: &ScriptContext) -> Result<ScriptResult, DbError> {
        let lua = Lua::new();

        // Set up the Lua environment
        self.setup_lua_globals(&lua, db_name, context)?;

        // Execute the script
        let chunk = lua.load(&script.code);

        match chunk.eval::<LuaValue>() {
            Ok(result) => {
                // Convert Lua result to JSON
                let json_result = self.lua_to_json(&lua, result)?;
                Ok(ScriptResult {
                    status: 200,
                    body: json_result,
                    headers: std::collections::HashMap::new(),
                })
            }
            Err(e) => Err(DbError::InternalError(format!("Lua error: {}", e))),
        }
    }

    /// Set up global Lua objects and functions
    fn setup_lua_globals(&self, lua: &Lua, db_name: &str, context: &ScriptContext) -> Result<(), DbError> {
        let globals = lua.globals();

        // Create 'solidb' namespace (just for logging or utils now)
        let solidb = lua.create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create solidb table: {}", e)))?;

        // solidb.log(msg)
        let log_fn = lua.create_function(|_, msg: String| {
            tracing::info!("[Lua Script] {}", msg);
            Ok(())
        }).map_err(|e| DbError::InternalError(format!("Failed to create log function: {}", e)))?;
        solidb.set("log", log_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set log: {}", e)))?;

        // Set solidb global
        globals.set("solidb", solidb)
            .map_err(|e| DbError::InternalError(format!("Failed to set solidb global: {}", e)))?;

        // Create global 'db' object
        let db_handle = lua.create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create db table: {}", e)))?;
        db_handle.set("_name", db_name.to_string())
            .map_err(|e| DbError::InternalError(format!("Failed to set db name: {}", e)))?;

        // db:collection(name) -> collection handle
        let storage_ref = self.storage.clone();
        let current_db = db_name.to_string();

        let collection_fn = lua.create_function(move |lua, (_, coll_name): (LuaValue, String)| {
            let storage = storage_ref.clone();
            let db_name = current_db.clone();

            // Create collection handle table
            let coll_handle = lua.create_table()?;
            coll_handle.set("_db", db_name.clone())?;
            coll_handle.set("_name", coll_name.clone())?;

            // col:get(key)
            let storage_get = storage.clone();
            let db_get = db_name.clone();
            let coll_get = coll_name.clone();
            let get_fn = lua.create_function(move |lua, (_, key): (LuaValue, String)| {
                let db = storage_get.get_database(&db_get)
                    .map_err(|e| mlua::Error::external(e))?;
                let collection = db.get_collection(&coll_get)
                    .map_err(|e| mlua::Error::external(e))?;

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

                let db = storage_insert.get_database(&db_insert)
                    .map_err(|e| mlua::Error::external(e))?;
                let collection = db.get_collection(&coll_insert)
                    .map_err(|e| mlua::Error::external(e))?;

                let inserted = collection.insert(json_doc)
                    .map_err(|e| mlua::Error::external(e))?;

                json_to_lua(lua, &inserted.to_value())
            })?;
            coll_handle.set("insert", insert_fn)?;

            // col:update(key, doc)
            let storage_update = storage.clone();
            let db_update = db_name.clone();
            let coll_update = coll_name.clone();
            let update_fn = lua.create_function(move |lua, (_, key, doc): (LuaValue, String, LuaValue)| {
                let json_doc = lua_to_json_value(lua, doc)?;

                let db = storage_update.get_database(&db_update)
                    .map_err(|e| mlua::Error::external(e))?;
                let collection = db.get_collection(&coll_update)
                    .map_err(|e| mlua::Error::external(e))?;

                let updated = collection.update(&key, json_doc)
                    .map_err(|e| mlua::Error::external(e))?;

                json_to_lua(lua, &updated.to_value())
            })?;
            coll_handle.set("update", update_fn)?;

            // col:delete(key)
            let storage_delete = storage.clone();
            let db_delete = db_name.clone();
            let coll_delete = coll_name.clone();
            let delete_fn = lua.create_function(move |_, (_, key): (LuaValue, String)| {
                let db = storage_delete.get_database(&db_delete)
                    .map_err(|e| mlua::Error::external(e))?;
                let collection = db.get_collection(&coll_delete)
                    .map_err(|e| mlua::Error::external(e))?;

                collection.delete(&key)
                    .map_err(|e| mlua::Error::external(e))?;

                Ok(true)
            })?;
            coll_handle.set("delete", delete_fn)?;

            // col:all() - returns all documents
            let storage_all = storage.clone();
            let db_all = db_name.clone();
            let coll_all = coll_name.clone();
            let all_fn = lua.create_function(move |lua, _: LuaValue| {
                let db = storage_all.get_database(&db_all)
                    .map_err(|e| mlua::Error::external(e))?;
                let collection = db.get_collection(&coll_all)
                    .map_err(|e| mlua::Error::external(e))?;

                let docs: Vec<JsonValue> = collection.scan(None)
                    .into_iter()
                    .map(|doc| doc.to_value())
                    .collect();

                let result = lua.create_table()?;
                for (i, doc) in docs.iter().enumerate() {
                    result.set(i + 1, json_to_lua(lua, doc)?)?;
                }

                Ok(LuaValue::Table(result))
            })?;
            coll_handle.set("all", all_fn)?;

            // col:count()
            let storage_count = storage.clone();
            let db_count = db_name.clone();
            let coll_count = coll_name.clone();
            let count_fn = lua.create_function(move |_, _: LuaValue| {
                let db = storage_count.get_database(&db_count)
                    .map_err(|e| mlua::Error::external(e))?;
                let collection = db.get_collection(&coll_count)
                    .map_err(|e| mlua::Error::external(e))?;

                Ok(collection.count() as i64)
            })?;
            coll_handle.set("count", count_fn)?;

            Ok(LuaValue::Table(coll_handle))
        }).map_err(|e| DbError::InternalError(format!("Failed to create collection function: {}", e)))?;

        db_handle.set("collection", collection_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set collection function: {}", e)))?;

        // db:query(query, bind_vars) -> results
        let storage_query = self.storage.clone();
        let db_query = db_name.to_string();
        let query_fn = lua.create_function(move |lua, (_, query, bind_vars): (LuaValue, String, Option<LuaValue>)| {
            let storage = storage_query.clone();

            // Parse bind vars
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

            // Parse and execute query
            let query_ast = parse(&query)
                .map_err(|e| mlua::Error::external(DbError::BadRequest(e.to_string())))?;

            let executor = if bind_vars_map.is_empty() {
                QueryExecutor::with_database(&storage, db_query.clone())
            } else {
                QueryExecutor::with_database_and_bind_vars(&storage, db_query.clone(), bind_vars_map)
            };

            let results = executor.execute(&query_ast)
                .map_err(|e| mlua::Error::external(e))?;

            // Convert results to Lua table
            let result_table = lua.create_table()?;
            for (i, doc) in results.iter().enumerate() {
                result_table.set(i + 1, json_to_lua(lua, doc)?)?;
            }

            Ok(LuaValue::Table(result_table))
        }).map_err(|e| DbError::InternalError(format!("Failed to create query function: {}", e)))?;

        db_handle.set("query", query_fn.clone())
            .map_err(|e| DbError::InternalError(format!("Failed to set query function: {}", e)))?;



        globals.set("db", db_handle)
            .map_err(|e| DbError::InternalError(format!("Failed to set db global: {}", e)))?;

        // Create 'request' table with context info
        let request = lua.create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create request table: {}", e)))?;

        request.set("method", context.method.clone())
            .map_err(|e| DbError::InternalError(format!("Failed to set method: {}", e)))?;
        request.set("path", context.path.clone())
            .map_err(|e| DbError::InternalError(format!("Failed to set path: {}", e)))?;

        // Query params
        let query = lua.create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create query table: {}", e)))?;
        for (k, v) in &context.query_params {
            query.set(k.clone(), v.clone())
                .map_err(|e| DbError::InternalError(format!("Failed to set query param: {}", e)))?;
        }
        request.set("query", query)
            .map_err(|e| DbError::InternalError(format!("Failed to set query: {}", e)))?;

        // URL params
        let params = lua.create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create params table: {}", e)))?;
        for (k, v) in &context.params {
            params.set(k.clone(), v.clone())
                .map_err(|e| DbError::InternalError(format!("Failed to set param: {}", e)))?;
        }
        request.set("params", params)
            .map_err(|e| DbError::InternalError(format!("Failed to set params: {}", e)))?;

        // Headers
        let headers = lua.create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create headers table: {}", e)))?;
        for (k, v) in &context.headers {
            headers.set(k.clone(), v.clone())
                .map_err(|e| DbError::InternalError(format!("Failed to set header: {}", e)))?;
        }
        request.set("headers", headers)
            .map_err(|e| DbError::InternalError(format!("Failed to set headers: {}", e)))?;

        // Body
        if let Some(body) = &context.body {
            let body_lua = json_to_lua(&lua, body)
                .map_err(|e| DbError::InternalError(format!("Failed to convert body: {}", e)))?;
            request.set("body", body_lua)
                .map_err(|e| DbError::InternalError(format!("Failed to set body: {}", e)))?;
        }

        globals.set("request", request)
            .map_err(|e| DbError::InternalError(format!("Failed to set request global: {}", e)))?;

        // Create 'response' helper table
        let response = lua.create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create response table: {}", e)))?;

        // response.json(data) - helper to return JSON
        let json_fn = lua.create_function(|_, data: LuaValue| {
            Ok(data)
        }).map_err(|e| DbError::InternalError(format!("Failed to create json function: {}", e)))?;
        response.set("json", json_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set json: {}", e)))?;

        globals.set("response", response)
            .map_err(|e| DbError::InternalError(format!("Failed to set response global: {}", e)))?;

        Ok(())
    }

    /// Convert Lua value to JSON
    fn lua_to_json(&self, lua: &Lua, value: LuaValue) -> Result<JsonValue, DbError> {
        lua_to_json_value(lua, value)
            .map_err(|e| DbError::InternalError(format!("Failed to convert Lua to JSON: {}", e)))
    }
}

/// Result from script execution
#[derive(Debug)]
pub struct ScriptResult {
    pub status: u16,
    pub body: JsonValue,
    pub headers: std::collections::HashMap<String, String>,
}

/// Convert JSON value to Lua value
fn json_to_lua(lua: &Lua, json: &JsonValue) -> LuaResult<LuaValue> {
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

/// Convert Lua value to JSON value
fn lua_to_json_value(lua: &Lua, value: LuaValue) -> LuaResult<JsonValue> {
    match value {
        LuaValue::Nil => Ok(JsonValue::Null),
        LuaValue::Boolean(b) => Ok(JsonValue::Bool(b)),
        LuaValue::Integer(i) => Ok(JsonValue::Number(i.into())),
        LuaValue::Number(n) => {
            Ok(serde_json::Number::from_f64(n)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null))
        }
        LuaValue::String(s) => Ok(JsonValue::String(s.to_str()?.to_string())),
        LuaValue::Table(t) => {
            // Check if it's an array (sequential integer keys starting from 1)
            let mut is_array = true;
            let mut max_index = 0;

            for pair in t.clone().pairs::<LuaValue, LuaValue>() {
                let (k, _) = pair?;
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

            if is_array && max_index > 0 {
                let mut arr = Vec::with_capacity(max_index as usize);
                for i in 1..=max_index {
                    let v: LuaValue = t.get(i)?;
                    arr.push(lua_to_json_value(lua, v)?);
                }
                Ok(JsonValue::Array(arr))
            } else {
                let mut obj = serde_json::Map::new();
                for pair in t.pairs::<String, LuaValue>() {
                    let (k, v) = pair?;
                    obj.insert(k, lua_to_json_value(lua, v)?);
                }
                Ok(JsonValue::Object(obj))
            }
        }
        _ => Ok(JsonValue::Null), // Functions, userdata, etc. become null
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
