//! Lua Scripting Engine for Custom API Endpoints
//!
//! This module provides embedded Lua scripting capabilities that allow users
//! to create custom API endpoints with full access to database operations.

use mlua::{Lua, Result as LuaResult, Value as LuaValue, FromLua};
use tokio::sync::broadcast;
use serde_json::Value as JsonValue;
use std::sync::Arc;
use std::collections::HashMap;

use crate::error::DbError;
use crate::storage::StorageEngine;
use crate::sdbql::{parse, QueryExecutor};

// Crypto imports
use sha2::Digest;
use hmac::Mac;
use base64::Engine;
use rand::RngCore;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use argon2::password_hash::SaltString;
// Custom JWT implementation for scripting
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
struct Header {
    alg: String,
    typ: String,
}

impl Header {
    fn default() -> Self {
        Self {
            alg: "HS256".to_string(),
            typ: "JWT".to_string(),
        }
    }
}

#[derive(Debug)]
struct Validation;

impl Validation {
    fn default() -> Self {
        Self
    }
}

#[derive(Debug)]
struct EncodingKey(Vec<u8>);

impl EncodingKey {
    fn from_secret(secret: &[u8]) -> Self {
        Self(secret.to_vec())
    }
}

#[derive(Debug)]
struct DecodingKey(Vec<u8>);

impl DecodingKey {
    fn from_secret(secret: &[u8]) -> Self {
        Self(secret.to_vec())
    }
}

fn encode<T: serde::Serialize>(
    _header: &Header,
    claims: &T,
    key: &EncodingKey,
) -> Result<String, String> {
    // JWT Header: {"alg":"HS256","typ":"JWT"}
    let header = r#"{"alg":"HS256","typ":"JWT"}"#;

    // Base64url encode header
    let header_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(header);

    // Serialize and encode claims
    let claims_json = serde_json::to_string(claims)
        .map_err(|e| format!("JWT encode failed: {}", e))?;
    let claims_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims_json.as_bytes());

    // Create signing input
    let signing_input = format!("{}.{}", header_b64, claims_b64);

    // Sign with HMAC-SHA256
    let signature = sign_hmac_sha256(&signing_input, &key.0)?;

    // Combine into JWT format: header.claims.signature
    Ok(format!("{}.{}.{}", header_b64, claims_b64, signature))
}

fn decode<T: serde::de::DeserializeOwned>(
    token: &str,
    key: &DecodingKey,
    _validation: &Validation,
) -> Result<TokenData<T>, String> {
    // Split JWT into parts
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("Invalid JWT format".to_string());
    }

    let (header_b64, claims_b64, signature_b64) = (parts[0], parts[1], parts[2]);

    // Verify header (should be {"alg":"HS256","typ":"JWT"})
    let header_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(header_b64)
        .map_err(|_| "Invalid JWT header".to_string())?;
    let header_str = String::from_utf8(header_bytes)
        .map_err(|_| "Invalid JWT header encoding".to_string())?;

    if !header_str.contains(r#""alg":"HS256""#) || !header_str.contains(r#""typ":"JWT""#) {
        return Err("Unsupported JWT algorithm or type".to_string());
    }

    // Verify signature
    let signing_input = format!("{}.{}", header_b64, claims_b64);
    let expected_signature = sign_hmac_sha256(&signing_input, &key.0)?;

    if expected_signature != signature_b64 {
        return Err("Invalid JWT signature".to_string());
    }

    // Decode claims
    let claims_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(claims_b64)
        .map_err(|_| "Invalid JWT claims".to_string())?;

    let claims: T = serde_json::from_slice(&claims_bytes)
        .map_err(|_| "Invalid JWT claims format".to_string())?;

    Ok(TokenData { header: Header::default(), claims })
}

#[derive(Debug)]
struct TokenData<T> {
    #[allow(dead_code)]
    header: Header,
    claims: T,
}

fn sign_hmac_sha256(data: &str, secret: &[u8]) -> Result<String, String> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let mut mac = Hmac::<Sha256>::new_from_slice(secret)
        .map_err(|e| format!("HMAC init failed: {}", e))?;
    mac.update(data.as_bytes());

    let result = mac.finalize();
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(result.into_bytes()))
}

/// Context passed to Lua scripts containing request information
#[derive(Debug, Clone)]
pub struct ScriptContext {
    /// HTTP method (GET, POST, PUT, DELETE)
    pub method: String,
    /// Request path (after /api/custom/)
    pub path: String,
    /// Query parameters
    pub query_params: HashMap<String, String>,
    /// URL parameters (e.g., :id)
    pub params: HashMap<String, String>,
    /// Request headers
    pub headers: HashMap<String, String>,
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
    pub updated_at: String,
}

fn default_database() -> String {
    "_system".to_string()
}

/// Lua scripting engine
pub struct ScriptEngine {
    storage: Arc<StorageEngine>,
    queue_notifier: Option<broadcast::Sender<()>>,
}

impl ScriptEngine {
    /// Create a new script engine with access to the storage layer
    pub fn new(storage: Arc<StorageEngine>) -> Self {
        Self { storage, queue_notifier: None }
    }

    pub fn with_queue_notifier(mut self, notifier: broadcast::Sender<()>) -> Self {
        self.queue_notifier = Some(notifier);
        self
    }

    /// Execute a Lua script with the given context
    pub async fn execute(&self, script: &Script, db_name: &str, context: &ScriptContext) -> Result<ScriptResult, DbError> {
        let lua = Lua::new();

        // Secure environment: Remove unsafe standard libraries and functions
        let globals = lua.globals();
        globals.set("os", LuaValue::Nil).map_err(|e| DbError::InternalError(format!("Failed to secure os: {}", e)))?;
        globals.set("io", LuaValue::Nil).map_err(|e| DbError::InternalError(format!("Failed to secure io: {}", e)))?;
        globals.set("debug", LuaValue::Nil).map_err(|e| DbError::InternalError(format!("Failed to secure debug: {}", e)))?;
        globals.set("package", LuaValue::Nil).map_err(|e| DbError::InternalError(format!("Failed to secure package: {}", e)))?;
        globals.set("dofile", LuaValue::Nil).map_err(|e| DbError::InternalError(format!("Failed to secure dofile: {}", e)))?;
        globals.set("load", LuaValue::Nil).map_err(|e| DbError::InternalError(format!("Failed to secure load: {}", e)))?;
        globals.set("loadfile", LuaValue::Nil).map_err(|e| DbError::InternalError(format!("Failed to secure loadfile: {}", e)))?;
        globals.set("require", LuaValue::Nil).map_err(|e| DbError::InternalError(format!("Failed to secure require: {}", e)))?;

        // Set up the Lua environment
        self.setup_lua_globals(&lua, db_name, context)?;

        // Execute the script
        let chunk = lua.load(&script.code);

        match chunk.eval_async::<LuaValue>().await {
            Ok(result) => {
                // Convert Lua result to JSON
                let json_result = self.lua_to_json(&lua, result)?;
                Ok(ScriptResult {
                    status: 200,
                    body: json_result,
                    headers: HashMap::new(),
                })
            }
            Err(e) => Err(DbError::InternalError(format!("Lua error: {}", e))),
        }
    }

    /// Set up global Lua objects and functions
    fn setup_lua_globals(&self, lua: &Lua, db_name: &str, context: &ScriptContext) -> Result<(), DbError> {
        let globals = lua.globals();

        // Create 'solidb' namespace
        let solidb = lua.create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create solidb table: {}", e)))?;

        // solidb.log(msg)
        let log_fn = lua.create_function(|_, msg: String| {
            tracing::info!("[Lua Script] {}", msg);
            Ok(())
        }).map_err(|e| DbError::InternalError(format!("Failed to create log function: {}", e)))?;
        solidb.set("log", log_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set log: {}", e)))?;

        // solidb.now() -> Unix timestamp
        let now_fn = lua.create_function(|_, (): ()| {
            Ok(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs())
        }).map_err(|e| DbError::InternalError(format!("Failed to create now function: {}", e)))?;
        solidb.set("now", now_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set now: {}", e)))?;

        // Extend string library with regex
        if let Ok(string_table) = globals.get::<mlua::Table>("string") {
            let regex_fn = lua.create_function(|_, (s, pattern): (String, String)| {
                let re = regex::Regex::new(&pattern).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
                Ok(re.is_match(&s))
            }).map_err(|e| DbError::InternalError(format!("Failed to create regex function: {}", e)))?;

            string_table.set("regex", regex_fn)
                .map_err(|e| DbError::InternalError(format!("Failed to set string.regex: {}", e)))?;

            // string.regex_replace(subject, pattern, replacement)
            let regex_replace_fn = lua.create_function(|_, (s, pattern, replacement): (String, String, String)| {
                let re = regex::Regex::new(&pattern).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
                Ok(re.replace_all(&s, replacement.as_str()).to_string())
            }).map_err(|e| DbError::InternalError(format!("Failed to create regex_replace function: {}", e)))?;

            string_table.set("regex_replace", regex_replace_fn)
                .map_err(|e| DbError::InternalError(format!("Failed to set string.regex_replace: {}", e)))?;
        }

        // solidb.fetch(url, options)
        let fetch_fn = lua.create_async_function(|lua, (url, options): (String, Option<LuaValue>)| async move {
            let client = reqwest::Client::new();
            let mut req_builder = client.get(&url); // Default to GET

            if let Some(opts) = options {
                if let LuaValue::Table(t) = opts {
                    // Method
                    if let Ok(method) = t.get::<String>("method") {
                        match method.to_uppercase().as_str() {
                            "POST" => req_builder = client.post(&url),
                            "PUT" => req_builder = client.put(&url),
                            "DELETE" => req_builder = client.delete(&url),
                            "PATCH" => req_builder = client.patch(&url),
                            "HEAD" => req_builder = client.head(&url),
                            _ => {} // Default GET
                        }
                    }

                    // Headers
                    if let Ok(headers) = t.get::<LuaValue>("headers") {
                        if let LuaValue::Table(h) = headers {
                            for pair in h.pairs::<String, String>() {
                                if let Ok((k, v)) = pair {
                                    req_builder = req_builder.header(k, v);
                                }
                            }
                        }
                    }

                    // Body
                    if let Ok(body) = t.get::<String>("body") {
                        req_builder = req_builder.body(body);
                    }
                }
            }

            match req_builder.send().await {
                Ok(res) => {
                    let status = res.status().as_u16();
                    let headers_map = res.headers().clone();
                    let text = res.text().await.unwrap_or_default();

                    let response_table = lua.create_table()?;
                    response_table.set("status", status)?;
                    response_table.set("body", text)?;
                    response_table.set("ok", status >= 200 && status < 300)?;

                    let resp_headers = lua.create_table()?;
                    for (k, v) in headers_map.iter() {
                         if let Ok(val_str) = v.to_str() {
                             resp_headers.set(k.as_str(), val_str)?;
                         }
                    }
                    response_table.set("headers", resp_headers)?;

                    Ok(response_table)
                }
                Err(e) => Err(mlua::Error::RuntimeError(format!("Fetch error: {}", e))),
            }
        }).map_err(|e| DbError::InternalError(format!("Failed to create fetch function: {}", e)))?;

        solidb.set("fetch", fetch_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set fetch: {}", e)))?;

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

        // db:transaction(callback) -> auto-commit/rollback transaction
        let storage_tx = self.storage.clone();
        let db_tx = db_name.to_string();
        let transaction_fn = lua.create_async_function(move |lua, (_, callback): (LuaValue, mlua::Function)| {
            let storage = storage_tx.clone();
            let db_name = db_tx.clone();

            async move {
                // Initialize transaction manager if needed
                storage.initialize_transactions()
                    .map_err(|e| mlua::Error::external(e))?;

                // Get transaction manager and begin transaction
                let tx_manager = storage.transaction_manager()
                    .map_err(|e| mlua::Error::external(e))?;

                let tx_id = tx_manager.begin(crate::transaction::IsolationLevel::ReadCommitted)
                    .map_err(|e| mlua::Error::external(e))?;

                // Create the transaction context table
                let tx_handle = lua.create_table()?;
                tx_handle.set("_tx_id", tx_id.to_string())?;
                tx_handle.set("_db", db_name.clone())?;

                // tx:collection(name) -> transactional collection handle
                let storage_coll = storage.clone();
                let tx_manager_coll = tx_manager.clone();
                let db_coll = db_name.clone();
                let tx_id_coll = tx_id;

                let tx_collection_fn = lua.create_function(move |lua, (_, coll_name): (LuaValue, String)| {
                    let storage = storage_coll.clone();
                    let tx_manager = tx_manager_coll.clone();
                    let db_name = db_coll.clone();
                    let tx_id = tx_id_coll;

                    // Create transactional collection handle
                    let coll_handle = lua.create_table()?;
                    coll_handle.set("_db", db_name.clone())?;
                    coll_handle.set("_name", coll_name.clone())?;
                    coll_handle.set("_tx_id", tx_id.to_string())?;

                    // col:insert(doc) - transactional insert
                    let storage_insert = storage.clone();
                    let tx_mgr_insert = tx_manager.clone();
                    let db_insert = db_name.clone();
                    let coll_insert = coll_name.clone();
                    let tx_id_insert = tx_id;
                    let insert_fn = lua.create_function(move |lua, (_, doc): (LuaValue, LuaValue)| {
                        let json_doc = lua_to_json_value(lua, doc)?;

                        let full_coll_name = format!("{}:{}", db_insert, coll_insert);
                        let collection = storage_insert.get_collection(&full_coll_name)
                            .map_err(|e| mlua::Error::external(e))?;

                        let tx_arc = tx_mgr_insert.get(tx_id_insert)
                            .map_err(|e| mlua::Error::external(e))?;
                        let mut tx = tx_arc.write().unwrap();
                        let wal = tx_mgr_insert.wal();

                        let inserted = collection.insert_tx(&mut tx, wal, json_doc)
                            .map_err(|e| mlua::Error::external(e))?;

                        json_to_lua(lua, &inserted.to_value())
                    })?;
                    coll_handle.set("insert", insert_fn)?;

                    // col:update(key, doc) - transactional update
                    let storage_update = storage.clone();
                    let tx_mgr_update = tx_manager.clone();
                    let db_update = db_name.clone();
                    let coll_update = coll_name.clone();
                    let tx_id_update = tx_id;
                    let update_fn = lua.create_function(move |lua, (_, key, doc): (LuaValue, String, LuaValue)| {
                        let json_doc = lua_to_json_value(lua, doc)?;

                        let full_coll_name = format!("{}:{}", db_update, coll_update);
                        let collection = storage_update.get_collection(&full_coll_name)
                            .map_err(|e| mlua::Error::external(e))?;

                        let tx_arc = tx_mgr_update.get(tx_id_update)
                            .map_err(|e| mlua::Error::external(e))?;
                        let mut tx = tx_arc.write().unwrap();
                        let wal = tx_mgr_update.wal();

                        let updated = collection.update_tx(&mut tx, wal, &key, json_doc)
                            .map_err(|e| mlua::Error::external(e))?;

                        json_to_lua(lua, &updated.to_value())
                    })?;
                    coll_handle.set("update", update_fn)?;

                    // col:delete(key) - transactional delete
                    let storage_delete = storage.clone();
                    let tx_mgr_delete = tx_manager.clone();
                    let db_delete = db_name.clone();
                    let coll_delete = coll_name.clone();
                    let tx_id_delete = tx_id;
                    let delete_fn = lua.create_function(move |_, (_, key): (LuaValue, String)| {
                        let full_coll_name = format!("{}:{}", db_delete, coll_delete);
                        let collection = storage_delete.get_collection(&full_coll_name)
                            .map_err(|e| mlua::Error::external(e))?;

                        let tx_arc = tx_mgr_delete.get(tx_id_delete)
                            .map_err(|e| mlua::Error::external(e))?;
                        let mut tx = tx_arc.write().unwrap();
                        let wal = tx_mgr_delete.wal();

                        collection.delete_tx(&mut tx, wal, &key)
                            .map_err(|e| mlua::Error::external(e))?;

                        Ok(true)
                    })?;
                    coll_handle.set("delete", delete_fn)?;

                    // col:get(key) - read (non-transactional, just reads current state)
                    let storage_get = storage.clone();
                    let db_get = db_name.clone();
                    let coll_get = coll_name.clone();
                    let get_fn = lua.create_function(move |lua, (_, key): (LuaValue, String)| {
                        let full_coll_name = format!("{}:{}", db_get, coll_get);
                        let collection = storage_get.get_collection(&full_coll_name)
                            .map_err(|e| mlua::Error::external(e))?;

                        match collection.get(&key) {
                            Ok(doc) => json_to_lua(lua, &doc.to_value()),
                            Err(crate::error::DbError::DocumentNotFound(_)) => Ok(LuaValue::Nil),
                            Err(e) => Err(mlua::Error::external(e)),
                        }
                    })?;
                    coll_handle.set("get", get_fn)?;

                    Ok(LuaValue::Table(coll_handle))
                })?;
                tx_handle.set("collection", tx_collection_fn)?;

                // Execute the callback with the transaction context
                let result = callback.call_async::<LuaValue>(LuaValue::Table(tx_handle)).await;

                match result {
                    Ok(value) => {
                        // Commit the transaction on success
                        storage.commit_transaction(tx_id)
                            .map_err(|e| mlua::Error::external(e))?;
                        Ok(value)
                    }
                    Err(e) => {
                        // Rollback on error
                        let _ = storage.rollback_transaction(tx_id);
                        Err(e)
                    }
                }
            }
        }).map_err(|e| DbError::InternalError(format!("Failed to create transaction function: {}", e)))?;

        db_handle.set("transaction", transaction_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set transaction function: {}", e)))?;

        // db:enqueue(queue, script, params, options)
        let storage_enqueue = self.storage.clone();
        let notifier_enqueue = self.queue_notifier.clone();
        let current_db_name = db_name.to_string();
        let enqueue_fn = lua.create_function(move |lua, args: mlua::MultiValue| {
            // Detect if called with colon (db:enqueue) or dot (db.enqueue)
            let (queue, script_path, params, options) = if args.len() >= 4 && matches!(args[0], LuaValue::Table(_)) {
                // Colon call: (self, queue, script, params, options)
                let q = String::from_lua(args.get(1).cloned().unwrap_or(LuaValue::Nil), lua)?;
                let s = String::from_lua(args.get(2).cloned().unwrap_or(LuaValue::Nil), lua)?;
                let p = args.get(3).cloned().unwrap_or(LuaValue::Nil);
                let o = args.get(4).cloned();
                (q, s, p, o)
            } else {
                // Dot call: (queue, script, params, options)
                let q = String::from_lua(args.get(0).cloned().unwrap_or(LuaValue::Nil), lua)?;
                let s = String::from_lua(args.get(1).cloned().unwrap_or(LuaValue::Nil), lua)?;
                let p = args.get(2).cloned().unwrap_or(LuaValue::Nil);
                let o = args.get(3).cloned();
                (q, s, p, o)
            };

            let json_params = lua_to_json_value(lua, params)?;
            
            let mut priority = 0;
            let mut max_retries = 20;
            let mut run_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            if let Some(LuaValue::Table(t)) = options {
                priority = t.get("priority").unwrap_or(0);
                max_retries = t.get("max_retries").unwrap_or(20);
                if let Ok(delay) = t.get::<u64>("run_at") {
                    run_at = delay;
                }
            }

            let job_id = uuid::Uuid::new_v4().to_string();
            let job = crate::queue::Job {
                id: job_id.clone(),
                revision: None,
                queue,
                priority,
                script_path,
                params: json_params,
                status: crate::queue::JobStatus::Pending,
                retry_count: 0,
                max_retries,
                last_error: None,
                cron_job_id: None,
                run_at,
                created_at: run_at,
                started_at: None,
                completed_at: None,
            };

            let db = storage_enqueue.get_database(&current_db_name)
                .map_err(|e| mlua::Error::external(e))?;
            
            // Ensure _jobs collection exists
            if db.get_collection("_jobs").is_err() {
                db.create_collection("_jobs".to_string(), None)
                    .map_err(|e| mlua::Error::external(e))?;
            }
            
            let jobs_coll = db.get_collection("_jobs")
                .map_err(|e| mlua::Error::external(e))?;

            let doc_val = serde_json::to_value(&job).unwrap();
            jobs_coll.insert(doc_val).map_err(|e| mlua::Error::external(e))?;

            // Notify worker
            if let Some(ref notifier) = notifier_enqueue {
                let _ = notifier.send(());
            }

            Ok(job_id)
        }).map_err(|e| DbError::InternalError(format!("Failed to create enqueue function: {}", e)))?;

        db_handle.set("enqueue", enqueue_fn)
            .map_err(|e| DbError::InternalError(format!("Failed to set enqueue function: {}", e)))?;

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

        // Create 'crypto' namespace
        let crypto = lua.create_table()
            .map_err(|e| DbError::InternalError(format!("Failed to create crypto table: {}", e)))?;

        // md5(data)
        let md5_fn = lua.create_function(|_, data: mlua::String| {
            let digest = md5::compute(&data.as_bytes());
            Ok(format!("{:x}", digest))
        }).map_err(|e| DbError::InternalError(format!("Failed to create md5 function: {}", e)))?;
        crypto.set("md5", md5_fn).map_err(|e| DbError::InternalError(format!("Failed to set md5: {}", e)))?;

        // sha256(data)
        let sha256_fn = lua.create_function(|_, data: mlua::String| {
            let mut hasher = sha2::Sha256::new();
            hasher.update(&data.as_bytes());
            Ok(hex::encode(hasher.finalize()))
        }).map_err(|e| DbError::InternalError(format!("Failed to create sha256 function: {}", e)))?;
        crypto.set("sha256", sha256_fn).map_err(|e| DbError::InternalError(format!("Failed to set sha256: {}", e)))?;

        // sha512(data)
        let sha512_fn = lua.create_function(|_, data: mlua::String| {
            let mut hasher = sha2::Sha512::new();
            hasher.update(&data.as_bytes());
            Ok(hex::encode(hasher.finalize()))
        }).map_err(|e| DbError::InternalError(format!("Failed to create sha512 function: {}", e)))?;
        crypto.set("sha512", sha512_fn).map_err(|e| DbError::InternalError(format!("Failed to set sha512: {}", e)))?;

        // hmac_sha256(key, data)
        let hmac_sha256_fn = lua.create_function(|_, (key, data): (mlua::String, mlua::String)| {
            type HmacSha256 = hmac::Hmac<sha2::Sha256>;
            let mut mac = HmacSha256::new_from_slice(&key.as_bytes())
                .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
            mac.update(&data.as_bytes());
            Ok(hex::encode(mac.finalize().into_bytes()))
        }).map_err(|e| DbError::InternalError(format!("Failed to create hmac_sha256 function: {}", e)))?;
        crypto.set("hmac_sha256", hmac_sha256_fn).map_err(|e| DbError::InternalError(format!("Failed to set hmac_sha256: {}", e)))?;

        // hmac_sha512(key, data)
        let hmac_sha512_fn = lua.create_function(|_, (key, data): (mlua::String, mlua::String)| {
            type HmacSha512 = hmac::Hmac<sha2::Sha512>;
            let mut mac = HmacSha512::new_from_slice(&key.as_bytes())
                .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
            mac.update(&data.as_bytes());
            Ok(hex::encode(mac.finalize().into_bytes()))
        }).map_err(|e| DbError::InternalError(format!("Failed to create hmac_sha512 function: {}", e)))?;
        crypto.set("hmac_sha512", hmac_sha512_fn).map_err(|e| DbError::InternalError(format!("Failed to set hmac_sha512: {}", e)))?;

        // base64_encode(data)
        let base64_encode_fn = lua.create_function(|_, data: mlua::String| {
            Ok(base64::engine::general_purpose::STANDARD.encode(&data.as_bytes()))
        }).map_err(|e| DbError::InternalError(format!("Failed to create base64_encode function: {}", e)))?;
        crypto.set("base64_encode", base64_encode_fn).map_err(|e| DbError::InternalError(format!("Failed to set base64_encode: {}", e)))?;

        // base64_decode(data)
        let base64_decode_fn = lua.create_function(|lua, data: String| {
            let bytes = base64::engine::general_purpose::STANDARD.decode(data)
                .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
            lua.create_string(&bytes)
        }).map_err(|e| DbError::InternalError(format!("Failed to create base64_decode function: {}", e)))?;
        crypto.set("base64_decode", base64_decode_fn).map_err(|e| DbError::InternalError(format!("Failed to set base64_decode: {}", e)))?;

        // base32_encode(data)
        let base32_encode_fn = lua.create_function(|_, data: mlua::String| {
            let encoded = base32::encode(base32::Alphabet::RFC4648 { padding: true }, &data.as_bytes());
            Ok(encoded)
        }).map_err(|e| DbError::InternalError(format!("Failed to create base32_encode function: {}", e)))?;
        crypto.set("base32_encode", base32_encode_fn).map_err(|e| DbError::InternalError(format!("Failed to set base32_encode: {}", e)))?;

        // base32_decode(data)
        let base32_decode_fn = lua.create_function(|lua, data: String| {
            let bytes = base32::decode(base32::Alphabet::RFC4648 { padding: true }, &data)
                .ok_or_else(|| mlua::Error::RuntimeError("Invalid base32".to_string()))?;
            lua.create_string(&bytes)
        }).map_err(|e| DbError::InternalError(format!("Failed to create base32_decode function: {}", e)))?;
        crypto.set("base32_decode", base32_decode_fn).map_err(|e| DbError::InternalError(format!("Failed to set base32_decode: {}", e)))?;

        // hex_encode(data)
        let hex_encode_fn = lua.create_function(|_, data: String| {
            Ok(hex::encode(data))
        }).map_err(|e| DbError::InternalError(format!("Failed to create hex_encode function: {}", e)))?;
        crypto.set("hex_encode", hex_encode_fn).map_err(|e| DbError::InternalError(format!("Failed to set hex_encode: {}", e)))?;

        // hex_decode(data)
        let hex_decode_fn = lua.create_function(|lua, data: String| {
            let bytes = hex::decode(data)
                .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
            lua.create_string(&bytes)
        }).map_err(|e| DbError::InternalError(format!("Failed to create hex_decode function: {}", e)))?;
        crypto.set("hex_decode", hex_decode_fn).map_err(|e| DbError::InternalError(format!("Failed to set hex_decode: {}", e)))?;

        // uuid()
        let uuid_fn = lua.create_function(|_, ()| {
            Ok(uuid::Uuid::new_v4().to_string())
        }).map_err(|e| DbError::InternalError(format!("Failed to create uuid function: {}", e)))?;
        crypto.set("uuid", uuid_fn).map_err(|e| DbError::InternalError(format!("Failed to set uuid: {}", e)))?;

        // uuid_v7()
        let uuid_v7_fn = lua.create_function(|_, ()| {
            Ok(uuid::Uuid::now_v7().to_string())
        }).map_err(|e| DbError::InternalError(format!("Failed to create uuid_v7 function: {}", e)))?;
        crypto.set("uuid_v7", uuid_v7_fn).map_err(|e| DbError::InternalError(format!("Failed to set uuid_v7: {}", e)))?;

        // random_bytes(len)
        let random_bytes_fn = lua.create_function(|lua, len: usize| {
            let mut bytes = vec![0u8; len];
            rand::thread_rng().fill_bytes(&mut bytes);
            lua.create_string(&bytes)
        }).map_err(|e| DbError::InternalError(format!("Failed to create random_bytes function: {}", e)))?;
        crypto.set("random_bytes", random_bytes_fn).map_err(|e| DbError::InternalError(format!("Failed to set random_bytes: {}", e)))?;

        // curve25519(secret, public_or_basepoint)
        let curve25519_fn = lua.create_function(|lua, (secret, public): (mlua::String, mlua::String)| {
             let secret_bytes = secret.as_bytes();
             if secret_bytes.len() != 32 {
                 return Err(mlua::Error::RuntimeError(format!("Secret must be 32 bytes, got {}", secret_bytes.len())));
             }
             let secret_slice: &[u8] = &secret_bytes;
             let secret_arr: [u8; 32] = secret_slice.try_into().unwrap();
             let secret_key = x25519_dalek::StaticSecret::from(secret_arr);

             let public_bytes = public.as_bytes();
             let public_slice: &[u8] = &public_bytes;
             if public_slice.len() == 32 {
                 // Shared secret calculation
                 let public_arr: [u8; 32] = public_slice.try_into().unwrap();
                 let public_key = x25519_dalek::PublicKey::from(public_arr);
                 let shared_secret = secret_key.diffie_hellman(&public_key);
                 lua.create_string(shared_secret.as_bytes())
             } else {
                 // Basepoint multiplication (Public Key generation)
                 let public_key = x25519_dalek::PublicKey::from(&secret_key);
                 lua.create_string(public_key.as_bytes())
             }
        }).map_err(|e| DbError::InternalError(format!("Failed to create curve25519 function: {}", e)))?;
        crypto.set("curve25519", curve25519_fn).map_err(|e| DbError::InternalError(format!("Failed to set curve25519: {}", e)))?;

        // hash_password(password)
        let hash_password_fn = lua.create_async_function(|_, password: String| async move {
            tokio::task::spawn_blocking(move || {
                let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
                let argon2 = Argon2::default();
                argon2.hash_password(password.as_bytes(), &salt)
                    .map(|h| h.to_string())
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))
            }).await.map_err(|e| mlua::Error::RuntimeError(e.to_string()))?
        }).map_err(|e| DbError::InternalError(format!("Failed to create hash_password function: {}", e)))?;
        crypto.set("hash_password", hash_password_fn).map_err(|e| DbError::InternalError(format!("Failed to set hash_password: {}", e)))?;

        // verify_password(hash, password)
        let verify_password_fn = lua.create_async_function(|_, (hash, password): (String, String)| async move {
            tokio::task::spawn_blocking(move || {
                let parsed_hash = PasswordHash::new(&hash)
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
                Ok(Argon2::default().verify_password(password.as_bytes(), &parsed_hash).is_ok())
            }).await.map_err(|e| mlua::Error::RuntimeError(e.to_string()))?
        }).map_err(|e| DbError::InternalError(format!("Failed to create verify_password function: {}", e)))?;
        crypto.set("verify_password", verify_password_fn).map_err(|e| DbError::InternalError(format!("Failed to set verify_password: {}", e)))?;

        // jwt_encode(claims, secret)
        let jwt_encode_fn = lua.create_function(move |lua, (claims, secret): (LuaValue, String)| -> Result<String, mlua::Error> {
            let json_claims = lua_to_json_value(lua, claims)?;
            let token = encode(
                &Header::default(),
                &json_claims,
                &EncodingKey::from_secret(secret.as_bytes()),
            ).map_err(|e| mlua::Error::RuntimeError(format!("JWT encode error: {}", e)))?;
            Ok(token)
        }).map_err(|e| DbError::InternalError(format!("Failed to create jwt_encode function: {}", e)))?;
        crypto.set("jwt_encode", jwt_encode_fn).map_err(|e| DbError::InternalError(format!("Failed to set jwt_encode: {}", e)))?;

        // jwt_decode(token, secret)
        let jwt_decode_fn = lua.create_function(move |lua, (token, secret): (String, String)| -> Result<mlua::Value, mlua::Error> {
            let token_data = decode::<serde_json::Value>(
                &token,
                &DecodingKey::from_secret(secret.as_bytes()),
                &Validation::default(),
            ).map_err(|e| mlua::Error::RuntimeError(format!("JWT decode error: {}", e)))?;

            json_to_lua(lua, &token_data.claims)
        }).map_err(|e| DbError::InternalError(format!("Failed to create jwt_decode function: {}", e)))?;
        crypto.set("jwt_decode", jwt_decode_fn).map_err(|e| DbError::InternalError(format!("Failed to set jwt_decode: {}", e)))?;

        globals.set("crypto", crypto)
            .map_err(|e| DbError::InternalError(format!("Failed to set crypto global: {}", e)))?;

        // Create 'time' namespace
        let time = lua.create_table().map_err(|e| DbError::InternalError(format!("Failed to create time table: {}", e)))?;

        // time.now() -> float (seconds)
        let now_fn = lua.create_function(|_, ()| {
            let now = chrono::Utc::now();
            let ts = now.timestamp() as f64 + now.timestamp_subsec_micros() as f64 / 1_000_000.0;
            Ok(ts)
        }).map_err(|e| DbError::InternalError(format!("Failed to create time.now function: {}", e)))?;
        time.set("now", now_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.now: {}", e)))?;

        // time.now_ms() -> int (milliseconds)
        let now_ms_fn = lua.create_function(|_, ()| {
            Ok(chrono::Utc::now().timestamp_millis())
        }).map_err(|e| DbError::InternalError(format!("Failed to create time.now_ms function: {}", e)))?;
        time.set("now_ms", now_ms_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.now_ms: {}", e)))?;

        // time.iso() -> string
        let iso_fn = lua.create_function(|_, ()| {
            Ok(chrono::Utc::now().to_rfc3339())
        }).map_err(|e| DbError::InternalError(format!("Failed to create time.iso function: {}", e)))?;
        time.set("iso", iso_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.iso: {}", e)))?;

        // time.sleep(ms) -> async
        let sleep_fn = lua.create_async_function(|_, ms: u64| async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(ms)).await;
            Ok(())
        }).map_err(|e| DbError::InternalError(format!("Failed to create time.sleep function: {}", e)))?;
        time.set("sleep", sleep_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.sleep: {}", e)))?;

        // time.format(ts, format) -> string
        let format_fn = lua.create_function(|_, (ts, fmt): (f64, String)| {
            let secs = ts.trunc() as i64;
            let nsecs = (ts.fract() * 1_000_000_000.0) as u32;
            let dt = chrono::DateTime::from_timestamp(secs, nsecs)
                .ok_or(mlua::Error::RuntimeError("Invalid timestamp".into()))?;
            Ok(dt.format(&fmt).to_string())
        }).map_err(|e| DbError::InternalError(format!("Failed to create time.format function: {}", e)))?;
        time.set("format", format_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.format: {}", e)))?;

        // time.parse(iso) -> float
        let parse_fn = lua.create_function(|_, iso: String| {
             let dt = chrono::DateTime::parse_from_rfc3339(&iso)
                 .map_err(|e| mlua::Error::RuntimeError(format!("Parse error: {}", e)))?;
             let ts = dt.timestamp() as f64 + dt.timestamp_subsec_micros() as f64 / 1_000_000.0;
             Ok(ts)
        }).map_err(|e| DbError::InternalError(format!("Failed to create time.parse function: {}", e)))?;
        time.set("parse", parse_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.parse: {}", e)))?;

        // time.add(ts, value, unit) -> float
        let add_fn = lua.create_function(|_, (ts, val, unit): (f64, f64, String)| {
             let added_seconds = match unit.as_str() {
                 "ms" => val / 1000.0,
                 "s" => val,
                 "m" => val * 60.0,
                 "h" => val * 3600.0,
                 "d" => val * 86400.0,
                 _ => return Err(mlua::Error::RuntimeError(format!("Unknown unit: {}", unit))),
             };
             Ok(ts + added_seconds)
        }).map_err(|e| DbError::InternalError(format!("Failed to create time.add function: {}", e)))?;
        time.set("add", add_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.add: {}", e)))?;

        // time.subtract(ts, value, unit) -> float
        let sub_fn = lua.create_function(|_, (ts, val, unit): (f64, f64, String)| {
             let sub_seconds = match unit.as_str() {
                 "ms" => val / 1000.0,
                 "s" => val,
                 "m" => val * 60.0,
                 "h" => val * 3600.0,
                 "d" => val * 86400.0,
                 _ => return Err(mlua::Error::RuntimeError(format!("Unknown unit: {}", unit))),
             };
             Ok(ts - sub_seconds)
        }).map_err(|e| DbError::InternalError(format!("Failed to create time.subtract function: {}", e)))?;
        time.set("subtract", sub_fn).map_err(|e| DbError::InternalError(format!("Failed to set time.subtract: {}", e)))?;

        globals.set("time", time).map_err(|e| DbError::InternalError(format!("Failed to set time global: {}", e)))?;

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
