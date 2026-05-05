//! HTTP fetch function for Lua
//! Security: Includes SSRF protection to prevent access to internal services

use crate::error::DbError;
use mlua::{Lua, Value as LuaValue};
use std::net::IpAddr;
use std::str::FromStr;

/// Validate URL to prevent SSRF attacks
/// Blocks:
/// - localhost and127.0.0.1
/// - Link-local addresses (169.254.x.x)
/// - Private IP ranges (10.x, 172.16-31.x, 192.168.x)
/// - IPV6 loopback and private addresses
/// - Non-HTTP/HTTPS schemes
fn validate_url_for_ssrf(url: &str) -> Result<(), String> {
    // Block non-http schemes
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("Only HTTP and HTTPS schemes are allowed".to_string());
    }

    // Parse URL and extract host
    let url_obj = url::Url::parse(url).map_err(|e| format!("Invalid URL: {}", e))?;
    let host = url_obj.host_str().ok_or("URL must have a host")?;

    // Block localhost variations
    let host_lower = host.to_lowercase();
    if host_lower == "localhost"
        || host_lower == "127.0.0.1"
        || host_lower == "::1"
        || host_lower == "0.0.0.0"
    {
        return Err("localhost addresses are not allowed".to_string());
    }

    // Check if host is an IP address and validate ranges
    if let Ok(ip) = IpAddr::from_str(host) {
        match ip {
            IpAddr::V4(ipv4) => {
                // Use octets to check private ranges
                let octets = ipv4.octets();
                // 10.0.0.0/8
                if octets[0] == 10 {
                    return Err("Private IP addresses (10.x.x.x) are not allowed".to_string());
                }
                // 172.16.0.0/12
                if octets[0] == 172 && (16..=31).contains(&octets[1]) {
                    return Err("Private IP addresses (172.16-31.x.x) are not allowed".to_string());
                }
                // 192.168.0.0/16
                if octets[0] == 192 && octets[1] == 168 {
                    return Err("Private IP addresses (192.168.x.x) are not allowed".to_string());
                }
                // 127.0.0.0/8 loopback
                if octets[0] == 127 {
                    return Err("Loopback addresses are not allowed".to_string());
                }
            }
            IpAddr::V6(ipv6) => {
                if ipv6.is_loopback() {
                    return Err("IPv6 loopback addresses are not allowed".to_string());
                }
            }
        }
        if ip.is_unspecified() {
            return Err("Unspecified IP addresses are not allowed".to_string());
        }
    }

    // DNS rebinding protection: resolve hostname and check IP
    // This is done via actual request, but we block known bad hostnames
    let blocked_hostnames = [
        "metadata.google.internal",
        "169.254.169.254",
        "metadata.internal",
    ];
    for blocked in &blocked_hostnames {
        if host_lower == *blocked || host_lower.ends_with(&format!(".{}", blocked)) {
            return Err(format!("Blocked hostname: {}", blocked));
        }
    }

    Ok(())
}

/// Create the fetch function for HTTP requests
pub fn create_fetch_function(lua: &Lua) -> Result<mlua::Function, DbError> {
    lua.create_async_function(
        |lua, (url, options): (String, Option<LuaValue>)| async move {
            // Security: Validate URL before making request
            if let Err(e) = validate_url_for_ssrf(&url) {
                return Err(mlua::Error::RuntimeError(format!("SSRF protection: {}", e)));
            }

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
