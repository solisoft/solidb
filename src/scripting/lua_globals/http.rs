//! HTTP fetch function for Lua
//! Security: Includes SSRF protection to prevent access to internal services

use crate::error::DbError;
use mlua::{Lua, Value as LuaValue};
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

/// Hostnames that resolve to cloud-provider metadata services. We refuse them
/// even before DNS resolution so misconfigured DNS can't surprise us.
const BLOCKED_HOSTNAMES: &[&str] = &[
    "metadata.google.internal",
    "metadata.internal",
    "instance-data",
];

/// Reject any IP that is not safely routable from a server: loopback,
/// private (RFC1918), CGNAT, link-local, broadcast, multicast, unspecified,
/// IPv6 ULA / link-local / loopback / unspecified, and IPv4-mapped IPv6
/// addresses (which we recursively check against the embedded v4 address).
fn validate_ip(ip: IpAddr) -> Result<(), String> {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            if v4.is_loopback() {
                return Err("loopback IP not allowed".into());
            }
            if v4.is_unspecified() {
                return Err("unspecified IP not allowed".into());
            }
            if v4.is_broadcast() {
                return Err("broadcast IP not allowed".into());
            }
            if v4.is_multicast() {
                return Err("multicast IP not allowed".into());
            }
            if v4.is_link_local() {
                return Err("link-local IP not allowed".into());
            }
            // RFC1918 private ranges
            if o[0] == 10
                || (o[0] == 172 && (16..=31).contains(&o[1]))
                || (o[0] == 192 && o[1] == 168)
            {
                return Err("private IP not allowed".into());
            }
            // 100.64.0.0/10 CGNAT
            if o[0] == 100 && (64..=127).contains(&o[1]) {
                return Err("CGNAT IP not allowed".into());
            }
            // 0.0.0.0/8 reserved
            if o[0] == 0 {
                return Err("reserved IP not allowed".into());
            }
            Ok(())
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return validate_ip(IpAddr::V4(v4));
            }
            if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
                return Err("special-use IPv6 not allowed".into());
            }
            let seg0 = v6.segments()[0];
            // fe80::/10 link-local
            if (seg0 & 0xffc0) == 0xfe80 {
                return Err("IPv6 link-local not allowed".into());
            }
            // fc00::/7 unique-local
            if (seg0 & 0xfe00) == 0xfc00 {
                return Err("IPv6 ULA not allowed".into());
            }
            Ok(())
        }
    }
}

/// Outcome of validating a fetch URL: the parsed URL, host, port to connect on,
/// and a fixed `SocketAddr` to bind for that hostname so reqwest can't redo DNS.
struct ValidatedTarget {
    url: url::Url,
    host: String,
    addr: SocketAddr,
}

/// Validate URL to prevent SSRF attacks. Performs DNS resolution and rejects
/// the request if any resolved address is non-public; pins the connection to
/// the first validated address to defeat DNS rebinding mid-flight.
async fn validate_url_for_ssrf(url: &str) -> Result<ValidatedTarget, String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("only http and https schemes are allowed".into());
    }
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid URL: {}", e))?;
    let host = parsed.host_str().ok_or("URL must have a host")?.to_string();
    let host_lower = host.to_lowercase();

    if host_lower == "localhost" {
        return Err("localhost not allowed".into());
    }
    if BLOCKED_HOSTNAMES
        .iter()
        .any(|b| host_lower == *b || host_lower.ends_with(&format!(".{}", b)))
    {
        return Err(format!("blocked hostname: {}", host_lower));
    }

    let port = parsed.port_or_known_default().ok_or("missing port")?;

    // If host is a literal IP we still validate it.
    if let Ok(ip) = IpAddr::from_str(&host) {
        validate_ip(ip)?;
        return Ok(ValidatedTarget {
            url: parsed,
            host,
            addr: SocketAddr::new(ip, port),
        });
    }

    // Resolve and validate every returned IP. We refuse the request if *any*
    // resolved address is non-public — preventing both selection bias and a
    // rebind that flips between safe and unsafe answers.
    let lookup = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| format!("DNS resolution failed for {}: {}", host, e))?;
    let addrs: Vec<SocketAddr> = lookup.collect();
    if addrs.is_empty() {
        return Err(format!("no addresses resolved for {}", host));
    }
    for sa in &addrs {
        validate_ip(sa.ip())?;
    }
    Ok(ValidatedTarget {
        url: parsed,
        host,
        addr: addrs[0],
    })
}

/// Create the fetch function for HTTP requests
pub fn create_fetch_function(lua: &Lua) -> Result<mlua::Function, DbError> {
    lua.create_async_function(
        |lua, (url, options): (String, Option<LuaValue>)| async move {
            let target = validate_url_for_ssrf(&url)
                .await
                .map_err(|e| mlua::Error::RuntimeError(format!("SSRF protection: {}", e)))?;

            // Pin DNS to the validated address — defeats DNS rebinding by ensuring
            // the connection goes to an address we already accepted.
            let client = reqwest::Client::builder()
                .resolve(&target.host, target.addr)
                .redirect(reqwest::redirect::Policy::none())
                // A peer that accepts and never answers would otherwise hold
                // the script — and its Lua state — indefinitely.
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| mlua::Error::RuntimeError(format!("HTTP client: {}", e)))?;
            let url = target.url.as_str().to_string();
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
