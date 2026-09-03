//! Shared host/IP checks used by Lua `fetch` and trigger webhooks.
//!
//! Two rules make this guard sound, and both used to be missing here:
//!
//! * **Parse the host, don't stringify it.** `Url::host_str` returns an IPv6
//!   host *with its brackets* (`"[::1]"`), which parses as neither an
//!   `IpAddr` nor a resolvable name — so every IPv6 literal fell through to
//!   the "unresolvable, allow it" arm. `http://[::1]:11434/` and
//!   `http://[::ffff:169.254.169.254]/` were accepted. `Url::host()` returns
//!   a typed `Host`, so the literal cases can't be missed.
//! * **Fail closed.** A DNS error or an empty answer used to return `Ok`,
//!   contradicting the doc comment. An attacker who can make resolution fail
//!   (or who simply names a host this resolver can't see) got a pass.
//!
//! [`validate_public_url_target`] additionally returns the address it
//! validated so the caller can pin the connection to it. Validating a name
//! and then letting the HTTP client resolve it again is a TOCTOU: a
//! low-TTL record that answers public on the first lookup and loopback on the
//! second defeats the check.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use url::Host;

const BLOCKED_HOSTNAMES: &[&str] = &[
    "metadata.google.internal",
    "metadata.internal",
    "instance-data",
];

fn validate_ipv4(v4: Ipv4Addr) -> Result<(), String> {
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
    if o[0] == 10 || (o[0] == 172 && (16..=31).contains(&o[1])) || (o[0] == 192 && o[1] == 168) {
        return Err("private IP not allowed".into());
    }
    if o[0] == 100 && (64..=127).contains(&o[1]) {
        return Err("CGNAT IP not allowed".into());
    }
    if o[0] == 0 {
        return Err("reserved IP not allowed".into());
    }
    // 192.0.0.0/24 — IETF protocol assignments, includes NAT64 well-known
    // prefixes and DS-Lite.
    if o[0] == 192 && o[1] == 0 && o[2] == 0 {
        return Err("IETF protocol-assignment IP not allowed".into());
    }
    // 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24 — documentation ranges.
    if (o[0] == 192 && o[1] == 0 && o[2] == 2)
        || (o[0] == 198 && o[1] == 51 && o[2] == 100)
        || (o[0] == 203 && o[1] == 0 && o[2] == 113)
    {
        return Err("documentation IP not allowed".into());
    }
    // 198.18.0.0/15 — benchmarking, routed inside some networks.
    if o[0] == 198 && (o[1] == 18 || o[1] == 19) {
        return Err("benchmarking IP not allowed".into());
    }
    // 240.0.0.0/4 — reserved; some stacks route it.
    if o[0] >= 240 {
        return Err("reserved IP not allowed".into());
    }
    Ok(())
}

fn validate_ipv6(v6: Ipv6Addr) -> Result<(), String> {
    if let Some(v4) = v6.to_ipv4_mapped() {
        return validate_ipv4(v4);
    }
    // `to_ipv4` also covers the deprecated IPv4-compatible form (::a.b.c.d).
    if let Some(v4) = v6.to_ipv4() {
        return validate_ipv4(v4);
    }
    if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
        return Err("special-use IPv6 not allowed".into());
    }
    let seg = v6.segments();
    if (seg[0] & 0xffc0) == 0xfe80 {
        return Err("IPv6 link-local not allowed".into());
    }
    if (seg[0] & 0xfe00) == 0xfc00 {
        return Err("IPv6 ULA not allowed".into());
    }
    // fec0::/10 — deprecated site-local, still routed by some stacks.
    if (seg[0] & 0xffc0) == 0xfec0 {
        return Err("IPv6 site-local not allowed".into());
    }
    // 64:ff9b::/96 and 64:ff9b:1::/48 — NAT64. The embedded IPv4 address is
    // what the traffic actually reaches, so validate that instead of the
    // wrapper.
    if seg[0] == 0x0064 && seg[1] == 0xff9b {
        let embedded = Ipv4Addr::new(
            (seg[6] >> 8) as u8,
            (seg[6] & 0xff) as u8,
            (seg[7] >> 8) as u8,
            (seg[7] & 0xff) as u8,
        );
        return validate_ipv4(embedded)
            .map_err(|e| format!("NAT64-embedded address rejected: {}", e));
    }
    // 2002::/16 — 6to4, which embeds an IPv4 address in segments 1-2.
    if seg[0] == 0x2002 {
        let embedded = Ipv4Addr::new(
            (seg[1] >> 8) as u8,
            (seg[1] & 0xff) as u8,
            (seg[2] >> 8) as u8,
            (seg[2] & 0xff) as u8,
        );
        return validate_ipv4(embedded)
            .map_err(|e| format!("6to4-embedded address rejected: {}", e));
    }
    Ok(())
}

pub fn validate_ip(ip: IpAddr) -> Result<(), String> {
    match ip {
        IpAddr::V4(v4) => validate_ipv4(v4),
        IpAddr::V6(v6) => validate_ipv6(v6),
    }
}

/// A URL whose host has been checked, plus the address it resolved to.
#[derive(Debug, Clone)]
pub struct ValidatedTarget {
    /// The host as it must be passed to `reqwest::ClientBuilder::resolve`
    /// (no brackets, even for IPv6).
    pub host: String,
    /// The address the caller must pin the connection to.
    pub addr: SocketAddr,
}

/// Check the hostname-level denylist shared by every caller.
fn check_hostname(host_lower: &str) -> Result<(), String> {
    if host_lower == "localhost" || host_lower.ends_with(".localhost") {
        return Err("localhost not allowed".into());
    }
    if BLOCKED_HOSTNAMES
        .iter()
        .any(|b| host_lower == *b || host_lower.ends_with(&format!(".{}", b)))
    {
        return Err(format!("blocked hostname: {}", host_lower));
    }
    Ok(())
}

/// Reject private/metadata hosts, and return the validated address so the
/// caller can pin its connection to it.
///
/// Resolves DNS and fails closed on a resolver error, an empty answer, or any
/// non-public address in the answer — refusing when *any* returned address is
/// non-public, so a rebind that mixes safe and unsafe answers is refused too.
pub fn validate_public_url_target(url: &url::Url) -> Result<ValidatedTarget, String> {
    let host = url.host().ok_or("URL must have a host")?;
    let port = url.port_or_known_default().ok_or("URL must have a port")?;

    match host {
        Host::Ipv4(v4) => {
            validate_ipv4(v4)?;
            Ok(ValidatedTarget {
                host: v4.to_string(),
                addr: SocketAddr::new(IpAddr::V4(v4), port),
            })
        }
        Host::Ipv6(v6) => {
            validate_ipv6(v6)?;
            Ok(ValidatedTarget {
                host: v6.to_string(),
                addr: SocketAddr::new(IpAddr::V6(v6), port),
            })
        }
        Host::Domain(domain) => {
            let host_lower = domain.to_lowercase();
            check_hostname(&host_lower)?;

            let addrs: Vec<SocketAddr> =
                std::net::ToSocketAddrs::to_socket_addrs(&(host_lower.as_str(), port))
                    .map_err(|e| format!("DNS resolution failed for {}: {}", host_lower, e))?
                    .collect();

            if addrs.is_empty() {
                return Err(format!("no addresses resolved for {}", host_lower));
            }
            for sa in &addrs {
                validate_ip(sa.ip())?;
            }
            Ok(ValidatedTarget {
                host: host_lower,
                addr: addrs[0],
            })
        }
    }
}

/// [`validate_public_url_target`] for callers that only need the verdict.
///
/// Prefer the target-returning form: without pinning, the address the client
/// finally connects to is not the one that was checked.
pub fn validate_public_url_host(url: &url::Url) -> Result<(), String> {
    validate_public_url_target(url).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(u: &str) -> Result<(), String> {
        validate_public_url_host(&url::Url::parse(u).expect("parse"))
    }

    /// The bracket bug: every one of these was accepted, because
    /// `host_str()` yields `"[::1]"` and the unresolvable-host arm returned
    /// `Ok`.
    #[test]
    fn ipv6_literals_are_rejected() {
        for u in [
            "http://[::1]:11434/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:169.254.169.254]/latest/meta-data/",
            "http://[fe80::1]/",
            "http://[fc00::1]/",
            "http://[fec0::1]/",
            "http://[::]/",
            "http://[64:ff9b::7f00:1]/",
            "http://[2002:7f00:1::]/",
        ] {
            assert!(check(u).is_err(), "should be rejected: {}", u);
        }
    }

    #[test]
    fn public_ipv6_is_allowed() {
        assert!(check("http://[2606:4700:4700::1111]/").is_ok());
    }

    #[test]
    fn ipv4_special_ranges_are_rejected() {
        for u in [
            "http://127.0.0.1/",
            "http://127.1/",
            "http://10.0.0.1/",
            "http://172.16.0.1/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://100.64.0.1/",
            "http://0.0.0.0/",
            "http://192.0.0.1/",
            "http://198.18.0.1/",
            "http://240.0.0.1/",
            "http://255.255.255.255/",
        ] {
            assert!(check(u).is_err(), "should be rejected: {}", u);
        }
    }

    #[test]
    fn public_ipv4_is_allowed() {
        assert!(check("http://1.1.1.1/").is_ok());
        assert!(check("http://93.184.216.34/").is_ok());
    }

    #[test]
    fn blocked_names_are_rejected() {
        for u in [
            "http://localhost/",
            "http://LOCALHOST/",
            "http://app.localhost/",
            "http://metadata.google.internal/",
            "http://x.metadata.internal/",
        ] {
            assert!(check(u).is_err(), "should be rejected: {}", u);
        }
    }

    /// Fail closed: an unresolvable name used to be accepted.
    #[test]
    fn unresolvable_names_are_rejected() {
        assert!(check("http://this-name-does-not-exist.invalid/").is_err());
    }

    /// The pinning contract: an IPv6 host is returned without brackets so it
    /// can be handed to `ClientBuilder::resolve`.
    #[test]
    fn validated_target_host_has_no_brackets() {
        let target =
            validate_public_url_target(&url::Url::parse("http://[2606:4700:4700::1111]/").unwrap())
                .expect("public address");
        assert_eq!(target.host, "2606:4700:4700::1111");
        assert_eq!(target.addr.port(), 80);
    }
}
