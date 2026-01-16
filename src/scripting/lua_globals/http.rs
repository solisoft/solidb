//! HTTP fetch function for Lua

use crate::error::DbError;
use mlua::{Lua, Value as LuaValue};

/// Create the fetch function for HTTP requests
pub fn create_fetch_function(lua: &Lua) -> Result<mlua::Function, DbError> {
    lua.create_async_function(
        |lua, (url, options): (String, Option<LuaValue>)| async move {
            let client = reqwest::Client::new();
            let mut req_builder = client.get(&url); // Default to GET

            if let Some(LuaValue::Table(t)) = options {
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
                if let Ok(LuaValue::Table(h)) = t.get::<LuaValue>("headers") {
                    for (k, v) in h.pairs::<String, String>().flatten() {
                        req_builder = req_builder.header(k, v);
                    }
                }

                // Body
                if let Ok(body) = t.get::<String>("body") {
                    req_builder = req_builder.body(body);
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
                    response_table.set("ok", (200..300).contains(&status))?;

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
        },
    )
    .map_err(|e| DbError::InternalError(format!("Failed to create fetch function: {}", e)))
}
