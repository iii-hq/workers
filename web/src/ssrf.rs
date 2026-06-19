//! SSRF defense for `web::fetch`. Parse + validate the URL, resolve the
//! host, and reject any address in a private / loopback / link-local /
//! multicast / reserved range. The resolve+validate happens once; the
//! socket is pinned to the validated IP (see fetch.rs) to defeat DNS
//! rebinding (TOCTOU between check and connect).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::OnceLock;

use ipnet::{Ipv4Net, Ipv6Net};

#[derive(Debug, Clone, Copy)]
pub struct SsrfPolicy {
    pub allow_loopback: bool,
}

#[derive(Debug, Clone)]
pub struct ParsedTarget {
    pub url: reqwest::Url,
    pub hostname: String,
    pub port: u16,
}

/// Parse + validate scheme and host shape. No DNS.
pub fn parse_target(raw: &str) -> Result<ParsedTarget, String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "url is not a valid absolute URL".to_string())?;
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!(
            "scheme not allowed: {scheme}: (only http: and https: are permitted)"
        ));
    }
    let hostname = {
        let h = url
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| "url has no hostname".to_string())?;
        // `host_str()` keeps the brackets on an IPv6 literal (e.g. "[::1]").
        // Strip them so downstream code can parse the string as an IpAddr.
        h.trim_start_matches('[').trim_end_matches(']').to_string()
    };
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "invalid port".to_string())?;
    Ok(ParsedTarget { url, hostname, port })
}

static V4_BLOCKLIST: OnceLock<Vec<(Ipv4Net, &'static str)>> = OnceLock::new();
static V6_BLOCKLIST: OnceLock<[(Ipv6Net, &'static str); 3]> = OnceLock::new();

fn v4_blocklist() -> &'static [(Ipv4Net, &'static str)] {
    V4_BLOCKLIST.get_or_init(|| {
        [
            ("0.0.0.0/8", "this-network"),
            ("10.0.0.0/8", "private rfc1918"),
            ("100.64.0.0/10", "cgnat rfc6598"),
            ("127.0.0.0/8", "loopback"),
            ("169.254.0.0/16", "link-local (incl. AWS metadata)"),
            ("172.16.0.0/12", "private rfc1918"),
            ("192.0.0.0/24", "ietf protocol assignments"),
            ("192.0.2.0/24", "documentation"),
            ("192.168.0.0/16", "private rfc1918"),
            ("198.18.0.0/15", "benchmarking"),
            ("198.51.100.0/24", "documentation"),
            ("203.0.113.0/24", "documentation"),
            ("224.0.0.0/4", "multicast"),
            ("240.0.0.0/4", "reserved"),
        ]
        .into_iter()
        .map(|(cidr, label)| (cidr.parse::<Ipv4Net>().expect("valid v4 cidr"), label))
        .collect()
    })
}

fn v6_blocklist() -> &'static [(Ipv6Net, &'static str)] {
    V6_BLOCKLIST.get_or_init(|| {
        [
            ("fe80::/10".parse().unwrap(), "link-local fe80::/10"),
            ("fc00::/7".parse().unwrap(), "unique-local fc00::/7"),
            ("ff00::/8".parse().unwrap(), "multicast ff00::/8"),
        ]
    })
}

fn check_ipv4(addr: Ipv4Addr, policy: &SsrfPolicy) -> Option<&'static str> {
    for (net, label) in v4_blocklist() {
        if *label == "loopback" && policy.allow_loopback {
            continue;
        }
        if net.contains(&addr) {
            return Some(label);
        }
    }
    None
}

fn check_ipv6(addr: Ipv6Addr, policy: &SsrfPolicy) -> Option<&'static str> {
    // IPv4-mapped (::ffff:a.b.c.d) routes to v4 — delegate to the v4 check.
    if let Some(v4) = addr.to_ipv4_mapped() {
        return check_ipv4(v4, policy);
    }
    if addr == Ipv6Addr::LOCALHOST {
        return if policy.allow_loopback { None } else { Some("loopback") };
    }
    if addr == Ipv6Addr::UNSPECIFIED {
        return Some("unspecified");
    }
    for (net, label) in v6_blocklist() {
        if net.contains(&addr) {
            return Some(label);
        }
    }
    None
}

/// Returns `Some(label)` if the address is blocked, `None` if allowed.
pub fn check_ip(addr: IpAddr, policy: &SsrfPolicy) -> Option<&'static str> {
    match addr {
        IpAddr::V4(v4) => check_ipv4(v4, policy),
        IpAddr::V6(v6) => check_ipv6(v6, policy),
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    pub address: IpAddr,
    pub hostname: String,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct SsrfReject {
    pub code: &'static str,
    pub message: String,
}

fn loopback_hint(policy: &SsrfPolicy) -> &'static str {
    if policy.allow_loopback {
        ""
    } else {
        " (set web.allow_loopback=true in config.yaml if loopback is intentional in this environment)"
    }
}

/// Resolve the host (or accept a literal IP), validate EVERY resolved
/// address against the blocklist, and return the address to dial (the
/// first resolved address). If ANY address is blocked, the whole call is
/// refused. Fail-closed: DNS failure / empty result map to `blocked_host`.
pub async fn check_target(
    target: &ParsedTarget,
    policy: &SsrfPolicy,
) -> Result<ResolvedTarget, SsrfReject> {
    let host = &target.hostname;

    // Literal IP short-circuit: skip DNS. Brackets were already stripped
    // from IPv6 literals in parse_target.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if let Some(label) = check_ip(ip, policy) {
            let hint = if label == "loopback" { loopback_hint(policy) } else { "" };
            return Err(SsrfReject {
                code: "blocked_host",
                message: format!("address {host} is in {label}{hint}"),
            });
        }
        return Ok(ResolvedTarget { address: ip, hostname: host.clone(), port: target.port });
    }

    let resolved: Vec<SocketAddr> = match tokio::net::lookup_host((host.as_str(), target.port)).await
    {
        Ok(it) => it.collect(),
        Err(e) => {
            return Err(SsrfReject {
                code: "blocked_host",
                message: format!("dns lookup failed for {host}: {e}"),
            });
        }
    };
    if resolved.is_empty() {
        return Err(SsrfReject {
            code: "blocked_host",
            message: format!("dns returned no addresses for {host}"),
        });
    }

    for sa in &resolved {
        if let Some(label) = check_ip(sa.ip(), policy) {
            let hint = if label == "loopback" { loopback_hint(policy) } else { "" };
            return Err(SsrfReject {
                code: "blocked_host",
                message: format!(
                    "{host} resolves to {} ({label}); refusing to dial{hint}",
                    sa.ip()
                ),
            });
        }
    }

    Ok(ResolvedTarget {
        address: resolved[0].ip(),
        hostname: host.clone(),
        port: target.port,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn blocked(ip: &str, allow_loopback: bool) -> Option<&'static str> {
        check_ip(ip.parse::<IpAddr>().unwrap(), &SsrfPolicy { allow_loopback })
    }

    #[test]
    fn blocks_private_v4_ranges() {
        for ip in ["10.0.0.1", "172.16.5.5", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "224.0.0.1", "240.0.0.1"] {
            assert!(blocked(ip, true).is_some(), "{ip} should be blocked");
        }
    }

    #[test]
    fn allows_public_v4() {
        assert!(blocked("1.1.1.1", true).is_none());
        assert!(blocked("93.184.216.34", true).is_none());
    }

    #[test]
    fn loopback_policy_v4_and_v6() {
        assert!(blocked("127.0.0.1", true).is_none());
        assert!(blocked("127.0.0.1", false).is_some());
        assert!(blocked("::1", true).is_none());
        assert!(blocked("::1", false).is_some());
        // loopback=true still blocks other private ranges
        assert!(blocked("169.254.169.254", true).is_some());
        assert!(blocked("10.0.0.1", true).is_some());
    }

    #[test]
    fn blocks_v6_ranges() {
        for ip in ["::", "fe80::1", "febf::1", "fc00::1", "fd12::1", "ff02::1"] {
            assert!(blocked(ip, true).is_some(), "{ip} should be blocked");
        }
        assert!(blocked("2606:4700:4700::1111", true).is_none());
    }

    #[test]
    fn blocks_v4_mapped_v6_metadata() {
        // both textual and (parsed) hex forms collapse to 169.254.169.254
        assert!(blocked("::ffff:169.254.169.254", true).is_some());
        assert!(blocked("::ffff:a9fe:a9fe", true).is_some());
    }

    #[test]
    fn parse_target_rejects_non_http() {
        assert!(parse_target("ftp://example.com/").is_err());
        assert!(parse_target("file:///etc/passwd").is_err());
        assert!(parse_target("not a url").is_err());
    }

    #[test]
    fn parse_target_defaults_ports() {
        assert_eq!(parse_target("http://example.com/").unwrap().port, 80);
        assert_eq!(parse_target("https://example.com/").unwrap().port, 443);
        assert_eq!(parse_target("http://example.com:8080/").unwrap().port, 8080);
    }

    #[tokio::test]
    async fn check_target_literal_v4_public_ok() {
        let t = parse_target("http://1.1.1.1/").unwrap();
        let r = check_target(&t, &SsrfPolicy { allow_loopback: true }).await.unwrap();
        assert_eq!(r.address, "1.1.1.1".parse::<std::net::IpAddr>().unwrap());
        assert_eq!(r.port, 80);
    }

    #[tokio::test]
    async fn check_target_literal_v4_private_blocked() {
        let t = parse_target("http://169.254.169.254/").unwrap();
        let rej = check_target(&t, &SsrfPolicy { allow_loopback: true }).await.unwrap_err();
        assert_eq!(rej.code, "blocked_host");
    }

    #[tokio::test]
    async fn check_target_literal_v6_bracket_stripped_and_blocked() {
        let t = parse_target("http://[::1]/").unwrap();
        let rej = check_target(&t, &SsrfPolicy { allow_loopback: false }).await.unwrap_err();
        assert_eq!(rej.code, "blocked_host");
        assert!(rej.message.contains("allow_loopback"));
    }
}
