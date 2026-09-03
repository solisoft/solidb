//! Enhanced Lua HTTP Helper Methods
//!
//! This module provides HTTP utilities like redirects, cookies, caching,
//! and response helpers for Lua scripts in SoliDB.

use cookie::{Cookie as HttpCookie, SameSite};
use lru::LruCache;
use mlua::{Function, Lua, Result as LuaResult, Value as LuaValue};
use serde_json::Value as JsonValue;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
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

/// Parse an origin entry from `SOLIDB_ALLOWED_REDIRECT_ORIGINS` into
/// (scheme, host, port). Accepts forms `host`, `scheme://host`, `scheme://host:port`.
/// `host`-only entries match either http or https.
fn parse_allowed_origin(entry: &str) -> Option<(Option<String>, String, Option<u16>)> {
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }
    if entry.contains("://") {
        let parsed = url::Url::parse(entry).ok()?;
        let host = parsed.host_str()?.to_lowercase();
        Some((Some(parsed.scheme().to_string()), host, parsed.port()))
    } else {
        // Bare host (and optional :port)
        let (host, port) = match entry.rsplit_once(':') {
            Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => {
                (h.to_lowercase(), p.parse::<u16>().ok())
            }
            _ => (entry.to_lowercase(), None),
        };
        Some((None, host, port))
    }
}

/// Returns true iff `url` matches one of the configured allowed origins.
/// Match is by exact (scheme, host, port) — never substring.
fn redirect_url_allowed(url_str: &str, allowed: &[&str]) -> bool {
    let parsed = match url::Url::parse(url_str) {
        Ok(u) => u,
        Err(_) => return false,
    };
    let url_host = match parsed.host_str() {
        Some(h) => h.to_lowercase(),
        None => return false,
    };
    let url_scheme = parsed.scheme();
    let url_port = parsed.port_or_known_default();

    allowed.iter().any(|raw| {
        let (allowed_scheme, allowed_host, allowed_port) = match parse_allowed_origin(raw) {
            Some(t) => t,
            None => return false,
        };
        if allowed_host != url_host {
            return false;
        }
        if let Some(scheme) = &allowed_scheme {
            if scheme != url_scheme {
                return false;
            }
        }
        if let Some(port) = allowed_port {
            if Some(port) != url_port {
                return false;
            }
        }
        true
    })
}

/// Create solidb.redirect(url) -> error with redirect status function
pub fn create_redirect_function(lua: &Lua) -> LuaResult<Function> {
    lua.create_function(|_, url: String| {
        let allowed_origins = std::env::var("SOLIDB_ALLOWED_REDIRECT_ORIGINS").unwrap_or_default();
        let allowed_list: Vec<&str> = allowed_origins
            .split(',')
            .map(str::trim)
            .filter(|o| !o.is_empty())
            .collect();

        // Absolute URLs are checked against the allowlist when one is configured.
        // Relative paths and (when no allowlist is set) absolute URLs are passed through —
        // SEC-095 made the allowlist opt-in.
        let is_absolute = url.starts_with("http://") || url.starts_with("https://");
        if is_absolute && !allowed_list.is_empty() && !redirect_url_allowed(&url, &allowed_list) {
            return Err(mlua::Error::RuntimeError(
                "REDIRECT: Forbidden - redirect to untrusted domain".to_string(),
            ));
        }

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
        assert!(result.unwrap());
    }
}
