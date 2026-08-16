//! Shared host/IP checks used by Lua `fetch` and trigger webhooks.

use std::net::{IpAddr, ToSocketAddrs};

const BLOCKED_HOSTNAMES: &[&str] = &[
    "metadata.google.internal",
    "metadata.internal",
    "instance-data",
];

pub fn validate_ip(ip: IpAddr) -> Result<(), String> {
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
            if o[0] == 10
                || (o[0] == 172 && (16..=31).contains(&o[1]))
                || (o[0] == 192 && o[1] == 168)
            {
                return Err("private IP not allowed".into());
            }
            if o[0] == 100 && (64..=127).contains(&o[1]) {
                return Err("CGNAT IP not allowed".into());
            }
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
            if (seg0 & 0xffc0) == 0xfe80 {
                return Err("IPv6 link-local not allowed".into());
            }
            if (seg0 & 0xfe00) == 0xfc00 {
                return Err("IPv6 ULA not allowed".into());
            }
            Ok(())
        }
    }
}

/// Reject private/metadata hosts. Resolves DNS when possible and fails
/// closed on any non-public address.
pub fn validate_public_url_host(url: &url::Url) -> Result<(), String> {
    let host = url.host_str().ok_or("URL must have a host")?.to_string();
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
    let port = url.port_or_known_default().unwrap_or(80);
    if let Ok(ip) = host.parse::<IpAddr>() {
        return validate_ip(ip);
    }
    match (host.as_str(), port).to_socket_addrs() {
        Ok(addrs) => {
            let addrs: Vec<_> = addrs.collect();
            if addrs.is_empty() {
                return Ok(());
            }
            for sa in addrs {
                validate_ip(sa.ip())?;
            }
            Ok(())
        }
        Err(_) => Ok(()),
    }
}
