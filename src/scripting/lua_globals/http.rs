//! HTTP fetch function for Lua
//! Security: Includes SSRF protection to prevent access to internal services

use crate::error::DbError;
use mlua::{Lua, Value as LuaValue};

/// Largest response body `fetch` reads, in bytes (`SOLIDB_LUA_FETCH_MAX_BYTES`,
/// default 10 MB). The body is held outside the Lua allocator, so the Lua
/// memory limit does not bound it.
fn fetch_max_bytes() -> usize {
    static MAX: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *MAX.get_or_init(|| {
        std::env::var("SOLIDB_LUA_FETCH_MAX_BYTES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(10 * 1024 * 1024)
    })
}

/// Validate `url` against the shared SSRF guard and return the address to
/// pin the connection to.
///
/// Audit L3: this module used to carry its own copy of the guard, which had
/// fallen behind `server::ssrf` (NAT64 `64:ff9b::/96`, 6to4,
/// IPv4-compatible, `fec0::/10`, `198.18/15`, `240/4`). The shared guard
/// resolves DNS synchronously, so it runs on the blocking pool.
async fn validate_fetch_target(
    url: &str,
) -> Result<(url::Url, crate::server::ssrf::ValidatedTarget), String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("only http and https schemes are allowed".into());
    }
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid URL: {}", e))?;
    let for_check = parsed.clone();
    let target = tokio::task::spawn_blocking(move || {
        crate::server::ssrf::validate_public_url_target(&for_check)
    })
    .await
    .map_err(|e| format!("validation task failed: {}", e))??;
    Ok((parsed, target))
}

/// Read a response body, refusing once it exceeds `max` bytes.
async fn read_capped_body(mut res: reqwest::Response, max: usize) -> Result<Vec<u8>, String> {
    if let Some(len) = res.content_length() {
        if len > max as u64 {
            return Err(format!("response body exceeds {} bytes", max));
        }
    }
    let mut body = Vec::new();
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|e| format!("reading body: {}", e))?
    {
        if body.len() + chunk.len() > max {
            return Err(format!("response body exceeds {} bytes", max));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Create the fetch function for HTTP requests
pub fn create_fetch_function(lua: &Lua) -> Result<mlua::Function, DbError> {
    lua.create_async_function(
        |lua, (url, options): (String, Option<LuaValue>)| async move {
            let (parsed, target) = validate_fetch_target(&url)
                .await
                .map_err(|e| mlua::Error::RuntimeError(format!("SSRF protection: {}", e)))?;

            // Pin DNS to the validated address — defeats DNS rebinding by ensuring
            // the connection goes to an address we already accepted. (A
            // literal-IP URL does no DNS; the override is then inert.)
            let client = reqwest::Client::builder()
                .resolve(&target.host, target.addr)
                .redirect(reqwest::redirect::Policy::none())
                // A peer that accepts and never answers would otherwise hold
                // the script — and its Lua state — indefinitely.
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| mlua::Error::RuntimeError(format!("HTTP client: {}", e)))?;
            let url = parsed.as_str().to_string();
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
                    let bytes = read_capped_body(res, fetch_max_bytes())
                        .await
                        .map_err(|e| mlua::Error::RuntimeError(format!("Fetch error: {}", e)))?;
                    let text = lua.create_string(&bytes)?;

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
