//! Enhanced Lua HTTP Helper Methods
//!
//! This module provides HTTP utilities like redirects, cookies, caching,
//! and response helpers for Lua scripts in SoliDB.

use cookie::{Cookie as HttpCookie, SameSite};
use lru::LruCache;
use mlua::{Function, Lua, Result as LuaResult, Value as LuaValue};
use serde_json::Value as JsonValue;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use time::{format_description, OffsetDateTime};

use crate::scripting::lua_to_json_value;

/// Global cache for HTTP caching
pub struct HttpCache {
    cache: Arc<Mutex<LruCache<String, CacheEntry>>>,
}

#[derive(Clone)]
struct CacheEntry {
    value: JsonValue,
    expires_at: SystemTime,
}

impl HttpCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: Arc::new(Mutex::new(LruCache::new(
                std::num::NonZeroUsize::new(capacity).unwrap(),
            ))),
        }
    }

    pub fn get(&self, key: &str) -> Option<JsonValue> {
        let mut cache = self.cache.lock().unwrap();
        if let Some(entry) = cache.get(key) {
            if entry.expires_at > SystemTime::now() {
                return Some(entry.value.clone());
            } else {
                cache.pop(key);
            }
        }
        None
    }

    pub fn set(&self, key: String, value: JsonValue, ttl_seconds: Option<u64>) {
        let mut cache = self.cache.lock().unwrap();
        let expires_at = if let Some(ttl) = ttl_seconds {
            SystemTime::now() + Duration::from_secs(ttl)
        } else {
            SystemTime::now() + Duration::from_secs(3600) // Default 1 hour
        };

        cache.put(key, CacheEntry { value, expires_at });
    }
}

/// Create solidb.redirect(url) -> error with redirect status function
pub fn create_redirect_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|_, url: String| {
        Err::<LuaValue, mlua::Error>(mlua::Error::RuntimeError(format!("REDIRECT:{}", url)))
    })
}

/// Create solidb.set_cookie(name, value, options) function
pub fn create_set_cookie_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(
        move |_lua, (name, value, options): (String, String, Option<LuaValue>)| {
            let mut cookie = HttpCookie::new(name, value);

            if let Some(LuaValue::Table(t)) = options {
                // Parse expires timestamp or ISO string
                if let Ok(expires) = t.get::<String>("expires") {
                    if let Ok(timestamp) = expires.parse::<i64>() {
                        if let Ok(datetime) = OffsetDateTime::from_unix_timestamp(timestamp) {
                            cookie.set_expires(datetime);
                        }
                    } else if let Ok(datetime) =
                        OffsetDateTime::parse(&expires, &format_description::well_known::Rfc3339)
                    {
                        cookie.set_expires(datetime);
                    }
                }

                // Path
                if let Ok(path) = t.get::<String>("path") {
                    cookie.set_path(path);
                }

                // Domain
                if let Ok(domain) = t.get::<String>("domain") {
                    cookie.set_domain(domain);
                }

                // Secure flag
                if let Ok(secure) = t.get::<bool>("secure") {
                    cookie.set_secure(secure);
                }

                // HttpOnly flag
                if let Ok(http_only) = t.get::<bool>("httpOnly") {
                    cookie.set_http_only(http_only);
                }

                // SameSite
                if let Ok(same_site) = t.get::<String>("sameSite") {
                    match same_site.as_str() {
                        "Strict" => cookie.set_same_site(SameSite::Strict),
                        "Lax" => cookie.set_same_site(SameSite::Lax),
                        "None" => cookie.set_same_site(SameSite::None),
                        _ => {}
                    }
                }
            }

            // Set the cookie as a special header that will be processed by the response handler
            let cookie_str = cookie.to_string();

            // This should be captured by the response system
            tracing::debug!("Setting cookie: {}", cookie_str);

            Ok(true)
        },
    )
}

/// Global HTTP cache singleton
fn get_http_cache() -> &'static HttpCache {
    use std::sync::OnceLock;
    static HTTP_CACHE: OnceLock<HttpCache> = OnceLock::new();
    HTTP_CACHE.get_or_init(|| HttpCache::new(1000))
}

/// Create solidb.cache(key, value, ttl_seconds) -> boolean function
pub fn create_cache_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(
        move |lua, (key, value, ttl): (String, LuaValue, Option<u64>)| {
            let json_value = lua_to_json_value(lua, value)?;
            get_http_cache().set(key, json_value, ttl);
            Ok(true)
        },
    )
}

/// Create solidb.cache_get(key) -> value function
pub fn create_cache_get_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(move |lua, key: String| {
        if let Some(value) = get_http_cache().get(&key) {
            json_to_lua(lua, &value)
        } else {
            Ok(LuaValue::Nil)
        }
    })
}

/// Create response.html(content) function
pub fn create_response_html_function(_lua: &Lua) -> LuaResult<Function> {
    let lua_ref = _lua;
    lua_ref.create_function(move |lua, content: String| {
        // Return a special marker that response system will understand
        Ok(LuaValue::String(
            lua.create_string(format!("HTML_RESPONSE:{}", content))
                .unwrap(),
        ))
    })
}

/// Create response.file(path) function
pub fn create_response_file_function(_lua: &Lua) -> LuaResult<Function> {
    let lua_ref = _lua;
    lua_ref.create_function(move |lua, path: String| {
        // Check if file exists and get its metadata
        match std::fs::metadata(&path) {
            Ok(metadata) => {
                let file_info = lua.create_table()?;
                file_info.set("path", path.clone())?;
                file_info.set("size", metadata.len())?;
                file_info.set("exists", true)?;

                if let Ok(modified) = metadata.modified() {
                    if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
                        file_info.set("modified", duration.as_secs())?;
                    }
                }

                Ok(LuaValue::Table(file_info))
            }
            Err(_) => {
                let file_info = lua.create_table()?;
                file_info.set("path", path)?;
                file_info.set("exists", false)?;
                Ok(LuaValue::Table(file_info))
            }
        }
    })
}

/// Create response.stream(data) function
pub fn create_response_stream_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|lua, data: LuaValue| {
        // Return a marker indicating streaming response
        let stream_info = lua.create_table()?;
        stream_info.set("type", "stream")?;
        stream_info.set("data", data)?;
        Ok(LuaValue::Table(stream_info))
    })
}

/// Create response.cors(options) function
pub fn create_response_cors_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|lua, options: Option<LuaValue>| {
        let cors_info = lua.create_table()?;

        if let Some(opts) = options {
            if let LuaValue::Table(t) = opts {
                // Origins
                if let Ok(origins) = t.get::<LuaValue>("origins") {
                    cors_info.set("origins", origins)?;
                }

                // Methods
                if let Ok(methods) = t.get::<LuaValue>("methods") {
                    cors_info.set("methods", methods)?;
                }

                // Headers
                if let Ok(headers) = t.get::<LuaValue>("headers") {
                    cors_info.set("headers", headers)?;
                }

                // Credentials
                if let Ok(credentials) = t.get::<bool>("credentials") {
                    cors_info.set("credentials", credentials)?;
                }

                // Max age
                if let Ok(max_age) = t.get::<u64>("max_age") {
                    cors_info.set("max_age", max_age)?;
                }
            }
        } else {
            // Default CORS settings
            cors_info.set("origins", "*")?;
            cors_info.set("methods", "GET, POST, PUT, DELETE, OPTIONS")?;
            cors_info.set("headers", "Content-Type, Authorization")?;
        }

        // Return CORS configuration that will be processed by response system
        Ok(LuaValue::Table(cors_info))
    })
}

/// Helper to convert JSON to Lua value
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

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    #[test]
    fn test_redirect_function() {
        let lua = Lua::new();
        let redirect_fn = create_redirect_function(&lua).unwrap();

        let result: Result<LuaValue, _> = redirect_fn.call("https://example.com");
        match result {
            Ok(_) => panic!("Expected error"),
            Err(e) => assert!(e.to_string().contains("REDIRECT:https://example.com")),
        }
    }

    #[test]
    fn test_cache_function() {
        let lua = Lua::new();
        let cache_fn = create_cache_function(&lua).unwrap();

        let data = lua.create_table().unwrap();
        data.set("test", "value").unwrap();

        let result: Result<bool, _> =
            cache_fn.call(("test_key".to_string(), LuaValue::Table(data), Some(60)));
        assert_eq!(result.unwrap(), true);
    }

    #[test]
    fn test_response_html() {
        let lua = Lua::new();
        let html_fn = create_response_html_function(&lua).unwrap();

        let result: Result<LuaValue, _> = html_fn.call("<h1>Test</h1>");
        match result {
            Ok(LuaValue::String(s)) => {
                assert!(s.to_str().unwrap().starts_with("HTML_RESPONSE:"));
            }
            _ => panic!("Expected string result"),
        }
    }
}
