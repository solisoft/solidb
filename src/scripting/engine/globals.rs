use std::sync::atomic::Ordering;
use mlua::{Value as LuaValue, Lua, FromLua};
use serde_json::Value as JsonValue;

use crate::error::DbError;
use crate::QueryExecutor;
use crate::sdbql::parser::parse;
use crate::scripting::types::ScriptContext;
use crate::scripting::conversion::{json_to_lua, lua_to_json_value, matches_filter};
use crate::scripting::{auth, ai_bindings, lua_globals};
use crate::scripting::dev_tools::*;
use crate::scripting::error_handling::*;
use crate::scripting::file_handling::*;
use crate::scripting::http_helpers::*;
use crate::scripting::validation::*;

use super::ScriptEngine;

pub fn setup_lua_globals(
    engine: &ScriptEngine,
    lua: &Lua,
    db_name: &str,
    context: &ScriptContext,
    script_info: Option<(&str, &str)>,
) -> Result<(), DbError> {
    let globals = lua.globals();

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

            tracing::info!("[Lua Script] {}", msg);

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
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create stats function: {}", e))
        })?;
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

    // Error handling functions
    let error_fn = create_error_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create error function: {}", e))
    })?;
    solidb
        .set("error", error_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set error: {}", e)))?;

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

    // Authentication & Authorization (solidb.auth namespace)
    let auth_table = auth::create_auth_table(lua, &context.user)
        .map_err(|e| DbError::InternalError(format!("Failed to create auth table: {}", e)))?;
    solidb
        .set("auth", auth_table)
        .map_err(|e| DbError::InternalError(format!("Failed to set auth: {}", e)))?;

    // File & Media Handling (using blob storage)
    let upload_fn = create_upload_function(lua, engine.storage.clone(), db_name.to_string())
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create upload function: {}", e))
        })?;
    solidb
        .set("upload", upload_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set upload: {}", e)))?;

    let file_info_fn =
        create_file_info_function(lua, engine.storage.clone(), db_name.to_string()).map_err(
            |e| DbError::InternalError(format!("Failed to create file_info function: {}", e)),
        )?;
    solidb
        .set("file_info", file_info_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set file_info: {}", e)))?;

    let file_read_fn =
        create_file_read_function(lua, engine.storage.clone(), db_name.to_string()).map_err(
            |e| DbError::InternalError(format!("Failed to create file_read function: {}", e)),
        )?;
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

    let file_list_fn =
        create_file_list_function(lua, engine.storage.clone(), db_name.to_string()).map_err(
            |e| DbError::InternalError(format!("Failed to create file_list function: {}", e)),
        )?;
    solidb
        .set("file_list", file_list_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set file_list: {}", e)))?;

    let image_process_fn = create_image_process_function(
        lua,
        engine.storage.clone(),
        db_name.to_string(),
    )
    .map_err(|e| {
        DbError::InternalError(format!("Failed to create image_process function: {}", e))
    })?;
    solidb
        .set("image_process", image_process_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set image_process: {}", e)))?;

    // Development Tools
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

    let dump_fn = create_dump_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create dump function: {}", e))
    })?;
    solidb
        .set("dump", dump_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set dump: {}", e)))?;

    // Set solidb global
    // Initialize solidb.env table
    let env_table = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create env table: {}", e)))?;

    // Populate env table from _env collection
    if let Ok(db) = engine.storage.get_database(&db_name) {
        if let Ok(collection) = db.get_collection("_env") {
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

    // Create 'streams' module
    if let Some(stream_manager) = engine.stream_manager.clone() {
        let streams_table = lua.create_table().map_err(|e| DbError::InternalError(format!("Failed to create streams table: {}", e)))?;

        // solidb.streams.list() -> array of {name: string, query: string, created_at: number}
        let manager_list = stream_manager.clone();
        let list_fn = lua.create_function(move |lua, (): ()| {
            let streams = manager_list.list_streams();
            let mut result = Vec::new();
            for stream in streams {
                let mut s = serde_json::Map::new();
                s.insert("name".to_string(), serde_json::Value::String(stream.name));
                // We might not want to expose full complex query object, maybe just source collection?
                // Or string representation if we had it.
                // For now, let's just expose created_at
                s.insert("created_at".to_string(), serde_json::Value::Number(serde_json::Number::from(stream.created_at)));
                result.push(serde_json::Value::Object(s));
            }
            
            // Use the json helper to convert to Lua table
            json_to_lua(lua, &serde_json::Value::Array(result))
        }).map_err(|e| DbError::InternalError(format!("Failed to create streams.list: {}", e)))?;
        
        streams_table.set("list", list_fn).map_err(|e| DbError::InternalError(format!("Failed to set streams.list: {}", e)))?;

        // solidb.streams.stop(name) -> void
        let manager_stop = stream_manager.clone();
        let stop_fn = lua.create_function(move |_, name: String| {
            manager_stop.stop_stream(&name).map_err(|e| mlua::Error::RuntimeError(e.to_string()))
        }).map_err(|e| DbError::InternalError(format!("Failed to create streams.stop: {}", e)))?;
        
        streams_table.set("stop", stop_fn).map_err(|e| DbError::InternalError(format!("Failed to set streams.stop: {}", e)))?;
        
        solidb.set("streams", streams_table).map_err(|e| DbError::InternalError(format!("Failed to set solidb.streams: {}", e)))?;
    }

    solidb
        .set("env", env_table)
        .map_err(|e| DbError::InternalError(format!("Failed to set solidb.env: {}", e)))?;

    // Add AI bindings (solidb.ai.*)
    let ai_table = ai_bindings::create_ai_table(&lua, engine.storage.clone(), db_name)
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

    // Create global 'db' object
    let db_handle = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create db table: {}", e)))?;
    db_handle
        .set("_name", db_name.to_string())
        .map_err(|e| DbError::InternalError(format!("Failed to set db name: {}", e)))?;

    // db:collection(name) -> collection handle
    let storage_ref = engine.storage.clone();
    let current_db = db_name.to_string();

    let collection_fn = lua
        .create_function(move |lua, (_, coll_name): (LuaValue, String)| {
            let storage = storage_ref.clone();
            let db_name = current_db.clone();

            // Create collection handle table
            let coll_handle = lua.create_table()?;
            coll_handle.set("_solidb_handle", true)?; // Marker to skip during session capture
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
            let insert_fn =
                lua.create_function(move |lua, (_, doc): (LuaValue, LuaValue)| {
                    let json_doc = lua_to_json_value(lua, doc)?;

                    let db = storage_insert
                        .get_database(&db_insert)
                        .map_err(mlua::Error::external)?;
                    let collection = db
                        .get_collection(&coll_insert)
                        .map_err(mlua::Error::external)?;

                    let inserted = collection
                        .insert(json_doc)
                        .map_err(mlua::Error::external)?;

                    json_to_lua(lua, &inserted.to_value())
                })?;
            coll_handle.set("insert", insert_fn)?;

            // col:update(key, doc)
            let storage_update = storage.clone();
            let db_update = db_name.clone();
            let coll_update = coll_name.clone();
            let update_fn = lua.create_function(
                move |lua, (_, key, doc): (LuaValue, String, LuaValue)| {
                    let json_doc = lua_to_json_value(lua, doc)?;

                    let db = storage_update
                        .get_database(&db_update)
                        .map_err(mlua::Error::external)?;
                    let collection = db
                        .get_collection(&coll_update)
                        .map_err(mlua::Error::external)?;

                    let updated = collection
                        .update(&key, json_doc)
                        .map_err(mlua::Error::external)?;

                    json_to_lua(lua, &updated.to_value())
                },
            )?;
            coll_handle.set("update", update_fn)?;

            // col:delete(key)
            let storage_delete = storage.clone();
            let db_delete = db_name.clone();
            let coll_delete = coll_name.clone();
            let delete_fn = lua.create_function(move |_, (_, key): (LuaValue, String)| {
                let db = storage_delete
                    .get_database(&db_delete)
                    .map_err(mlua::Error::external)?;
                let collection = db
                    .get_collection(&coll_delete)
                    .map_err(mlua::Error::external)?;

                collection
                    .delete(&key)
                    .map_err(mlua::Error::external)?;

                Ok(true)
            })?;
            coll_handle.set("delete", delete_fn)?;

            // col:count(filter?) - count all or matching documents
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
                            // Count matching documents
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

            // col:find(filter) - find documents matching filter
            let storage_find = storage.clone();
            let db_find = db_name.clone();
            let coll_find = coll_name.clone();
            let find_fn =
                lua.create_function(move |lua, (_, filter): (LuaValue, LuaValue)| {
                    let filter_json = lua_to_json_value(lua, filter)?;

                    let db = storage_find
                        .get_database(&db_find)
                        .map_err(mlua::Error::external)?;
                    let collection = db
                        .get_collection(&coll_find)
                        .map_err(mlua::Error::external)?;

                    // Scan all documents and filter
                    let all_docs = collection.scan(None);
                    let mut results = Vec::new();

                    for doc in all_docs {
                        let doc_value = doc.to_value();
                        if matches_filter(&doc_value, &filter_json) {
                            results.push(doc_value);
                        }
                    }

                    // Convert to Lua table
                    let result_table = lua.create_table()?;
                    for (i, doc) in results.iter().enumerate() {
                        result_table.set(i + 1, json_to_lua(lua, doc)?)?;
                    }

                    Ok(LuaValue::Table(result_table))
                })?;
            coll_handle.set("find", find_fn)?;

            // col:find_one(filter) - find first document matching filter
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

                    // Scan documents and return first match
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

            // col:bulk_insert(docs) - insert multiple documents
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
                        .get_collection(&coll_bulk)
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
                        let result = collection
                            .insert(doc)
                            .map_err(mlua::Error::external)?;
                        inserted.push(result.to_value());
                    }

                    // Return array of inserted documents
                    let result_table = lua.create_table()?;
                    for (i, doc) in inserted.iter().enumerate() {
                        result_table.set(i + 1, json_to_lua(lua, doc)?)?;
                    }

                    Ok(LuaValue::Table(result_table))
                })?;
            coll_handle.set("bulk_insert", bulk_insert_fn)?;

            // col:upsert(key_or_filter, doc) - insert or update
            // If key_or_filter is a string, it's treated as a _key lookup
            // If key_or_filter is a table, it's treated as a filter
            let storage_upsert = storage.clone();
            let db_upsert = db_name.clone();
            let coll_upsert = coll_name.clone();
            let upsert_fn = lua.create_function(
                move |lua, (_, key_or_filter, doc): (LuaValue, LuaValue, LuaValue)| {
                    let mut doc_json = lua_to_json_value(lua, doc)?;

                    let db = storage_upsert
                        .get_database(&db_upsert)
                        .map_err(mlua::Error::external)?;
                    let collection = db
                        .get_collection(&coll_upsert)
                        .map_err(mlua::Error::external)?;

                    // Check if key_or_filter is a string (key) or table (filter)
                    let existing_key: Option<String> = match &key_or_filter {
                        LuaValue::String(s) => {
                            let key = s.to_str()?.to_string();
                            // Check if document with this key exists
                            match collection.get(&key) {
                                Ok(_) => Some(key),
                                Err(_) => {
                                    // Set _key in doc for insert
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
                            // Find existing document by filter
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
                        // Update existing
                        collection
                            .update(&key, doc_json)
                            .map_err(mlua::Error::external)?
                            .to_value()
                    } else {
                        // Insert new
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

    db_handle.set("collection", collection_fn).map_err(|e| {
        DbError::InternalError(format!("Failed to set collection function: {}", e))
    })?;

    // db:query(query, bind_vars) -> results
    let storage_query = engine.storage.clone();
    let db_query = db_name.to_string();
    let query_fn = lua
        .create_function(
            move |lua, (_, query, bind_vars): (LuaValue, String, Option<LuaValue>)| {
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
                    QueryExecutor::with_database_and_bind_vars(
                        &storage,
                        db_query.clone(),
                        bind_vars_map,
                    )
                };

                let results = executor
                    .execute(&query_ast)
                    .map_err(mlua::Error::external)?;

                // Convert results to Lua table
                let result_table = lua.create_table()?;
                for (i, doc) in results.iter().enumerate() {
                    result_table.set(i + 1, json_to_lua(lua, doc)?)?;
                }

                Ok(LuaValue::Table(result_table))
            },
        )
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create query function: {}", e))
        })?;

    db_handle
        .set("query", query_fn.clone())
        .map_err(|e| DbError::InternalError(format!("Failed to set query function: {}", e)))?;

    // db:transaction(callback) -> auto-commit/rollback transaction
    let storage_tx = engine.storage.clone();
    let db_tx = db_name.to_string();
    let transaction_fn = lua
        .create_async_function(move |lua, (_, callback): (LuaValue, mlua::Function)| {
            let storage = storage_tx.clone();
            let db_name = db_tx.clone();

            async move {
                // Initialize transaction manager if needed
                storage
                    .initialize_transactions()
                    .map_err(mlua::Error::external)?;

                // Get transaction manager and begin transaction
                let tx_manager = storage
                    .transaction_manager()
                    .map_err(mlua::Error::external)?;

                let tx_id = tx_manager
                    .begin(crate::transaction::IsolationLevel::ReadCommitted)
                    .map_err(mlua::Error::external)?;

                // Create the transaction context table
                let tx_handle = lua.create_table()?;
                tx_handle.set("_tx_id", tx_id.to_string())?;
                tx_handle.set("_db", db_name.clone())?;

                // tx:collection(name) -> transactional collection handle
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
                        let insert_fn =
                            lua.create_function(move |lua, (_, doc): (LuaValue, LuaValue)| {
                                let json_doc = lua_to_json_value(lua, doc)?;

                                let full_coll_name = format!("{}:{}", db_insert, coll_insert);
                                let collection = storage_insert
                                    .get_collection(&full_coll_name)
                                    .map_err(mlua::Error::external)?;

                                let tx_arc = tx_mgr_insert
                                    .get(tx_id_insert)
                                    .map_err(mlua::Error::external)?;
                                let mut tx = tx_arc.write().unwrap();
                                let wal = tx_mgr_insert.wal();

                                let inserted = collection
                                    .insert_tx(&mut tx, wal, json_doc)
                                    .map_err(mlua::Error::external)?;

                                json_to_lua(lua, &inserted.to_value())
                            })?;
                        coll_handle.set("insert", insert_fn)?;

                        // col:update(key, doc) - transactional update
                        let storage_update = storage.clone();
                        let tx_mgr_update = tx_manager.clone();
                        let db_update = db_name.clone();
                        let coll_update = coll_name.clone();
                        let tx_id_update = tx_id;
                        let update_fn = lua.create_function(
                            move |lua, (_, key, doc): (LuaValue, String, LuaValue)| {
                                let json_doc = lua_to_json_value(lua, doc)?;

                                let full_coll_name = format!("{}:{}", db_update, coll_update);
                                let collection = storage_update
                                    .get_collection(&full_coll_name)
                                    .map_err(mlua::Error::external)?;

                                let tx_arc = tx_mgr_update
                                    .get(tx_id_update)
                                    .map_err(mlua::Error::external)?;
                                let mut tx = tx_arc.write().unwrap();
                                let wal = tx_mgr_update.wal();

                                let updated = collection
                                    .update_tx(&mut tx, wal, &key, json_doc)
                                    .map_err(mlua::Error::external)?;

                                json_to_lua(lua, &updated.to_value())
                            },
                        )?;
                        coll_handle.set("update", update_fn)?;

                        // col:delete(key) - transactional delete
                        let storage_delete = storage.clone();
                        let tx_mgr_delete = tx_manager.clone();
                        let db_delete = db_name.clone();
                        let coll_delete = coll_name.clone();
                        let tx_id_delete = tx_id;
                        let delete_fn =
                            lua.create_function(move |_, (_, key): (LuaValue, String)| {
                                let full_coll_name = format!("{}:{}", db_delete, coll_delete);
                                let collection = storage_delete
                                    .get_collection(&full_coll_name)
                                    .map_err(mlua::Error::external)?;

                                let tx_arc = tx_mgr_delete
                                    .get(tx_id_delete)
                                    .map_err(mlua::Error::external)?;
                                let mut tx = tx_arc.write().unwrap();
                                let wal = tx_mgr_delete.wal();

                                collection
                                    .delete_tx(&mut tx, wal, &key)
                                    .map_err(mlua::Error::external)?;

                                Ok(true)
                            })?;
                        coll_handle.set("delete", delete_fn)?;

                        // col:get(key) - read (non-transactional, just reads current state)
                        let storage_get = storage.clone();
                        let db_get = db_name.clone();
                        let coll_get = coll_name.clone();
                        let get_fn =
                            lua.create_function(move |lua, (_, key): (LuaValue, String)| {
                                let full_coll_name = format!("{}:{}", db_get, coll_get);
                                let collection = storage_get
                                    .get_collection(&full_coll_name)
                                    .map_err(mlua::Error::external)?;

                                match collection.get(&key) {
                                    Ok(doc) => json_to_lua(lua, &doc.to_value()),
                                    Err(crate::error::DbError::DocumentNotFound(_)) => {
                                        Ok(LuaValue::Nil)
                                    }
                                    Err(e) => Err(mlua::Error::external(e)),
                                }
                            })?;
                        coll_handle.set("get", get_fn)?;

                        Ok(LuaValue::Table(coll_handle))
                    })?;
                tx_handle.set("collection", tx_collection_fn)?;

                // Execute the callback with the transaction context
                let result = callback
                    .call_async::<LuaValue>(LuaValue::Table(tx_handle))
                    .await;

                match result {
                    Ok(value) => {
                        // Commit the transaction on success
                        storage
                            .commit_transaction(tx_id)
                            .map_err(mlua::Error::external)?;
                        Ok(value)
                    }
                    Err(e) => {
                        // Rollback on error
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

    // db:enqueue(queue, script, params, options)
    let storage_enqueue = engine.storage.clone();
    let notifier_enqueue = engine.queue_notifier.clone();
    let current_db_name = db_name.to_string();
    let enqueue_fn = lua
        .create_function(move |lua, args: mlua::MultiValue| {
            // Detect if called with colon (db:enqueue) or dot (db.enqueue)
            let (queue, script_path, params, options) = if args.len() >= 4
                && matches!(args[0], LuaValue::Table(_))
            {
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

            let db = storage_enqueue
                .get_database(&current_db_name)
                .map_err(mlua::Error::external)?;

            // Ensure _jobs collection exists
            if db.get_collection("_jobs").is_err() {
                db.create_collection("_jobs".to_string(), None)
                    .map_err(mlua::Error::external)?;
            }

            let jobs_coll = db
                .get_collection("_jobs")
                .map_err(mlua::Error::external)?;

            let doc_val = serde_json::to_value(&job).unwrap();
            jobs_coll
                .insert(doc_val)
                .map_err(mlua::Error::external)?;

            // Notify worker
            if let Some(ref notifier) = notifier_enqueue {
                let _ = notifier.send(());
            }

            Ok(job_id)
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create enqueue function: {}", e))
        })?;

    db_handle.set("enqueue", enqueue_fn).map_err(|e| {
        DbError::InternalError(format!("Failed to set enqueue function: {}", e))
    })?;

    globals
        .set("db", db_handle)
        .map_err(|e| DbError::InternalError(format!("Failed to set db global: {}", e)))?;

    // Create 'request' table with context info
    let request = lua.create_table().map_err(|e| {
        DbError::InternalError(format!("Failed to create request table: {}", e))
    })?;

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
    let headers = lua.create_table().map_err(|e| {
        DbError::InternalError(format!("Failed to create headers table: {}", e))
    })?;
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
        let body_lua = json_to_lua(&lua, body)
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

    // Create 'response' helper table
    let response = lua.create_table().map_err(|e| {
        DbError::InternalError(format!("Failed to create response table: {}", e))
    })?;

    // response.json(data) - helper to return JSON
    let json_fn = lua
        .create_function(|_, data: LuaValue| Ok(data))
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create json function: {}", e))
        })?;
    response
        .set("json", json_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set json: {}", e)))?;

    // response.html(content) - HTML response
    let html_fn = create_response_html_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create html function: {}", e))
    })?;
    response
        .set("html", html_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set html: {}", e)))?;

    // response.file(path) - file download
    let file_fn = create_response_file_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create file function: {}", e))
    })?;
    response
        .set("file", file_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set file: {}", e)))?;

    // response.stream(data) - streaming response
    let stream_fn = create_response_stream_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create stream function: {}", e))
    })?;
    response
        .set("stream", stream_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set stream: {}", e)))?;

    // response.cors(options) - CORS headers
    let cors_fn = create_response_cors_function(lua).map_err(|e| {
        DbError::InternalError(format!("Failed to create cors function: {}", e))
    })?;
    response
        .set("cors", cors_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set cors: {}", e)))?;

    globals
        .set("response", response)
        .map_err(|e| DbError::InternalError(format!("Failed to set response global: {}", e)))?;

    // Setup crypto namespace (md5, sha256, jwt, password hashing, etc.)
    lua_globals::setup_crypto_globals(lua)?;

    // Setup extended time namespace (now, sleep, format, parse, add, subtract)
    lua_globals::setup_time_ext_globals(lua)?;

    Ok(())
}
