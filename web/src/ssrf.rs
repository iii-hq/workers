//! SSRF defense for `web::fetch`. Parse + validate the URL, resolve the
//! host, and reject any address in a private / loopback / link-local /
//! multicast / reserved range. The resolve+validate happens once; the
//! socket is pinned to the validated IP (see fetch.rs) to defeat DNS
//! rebinding (TOCTOU between check and connect).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

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
    let hostname = url
        .host_str()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| "url has no hostname".to_string())?
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "invalid port".to_string())?;
    Ok(ParsedTarget { url, hostname, port })
}

fn v4_blocklist() -> Vec<(Ipv4Net, &'static str)> {
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
}

fn check_ipv4(addr: Ipv4Addr, policy: &SsrfPolicy) -> Option<&'static str> {
    for (net, label) in v4_blocklist() {
        if label == "loopback" && policy.allow_loopback {
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
    let blocks: [(Ipv6Net, &'static str); 3] = [
        ("fe80::/10".parse().unwrap(), "link-local fe80::/10"),
        ("fc00::/7".parse().unwrap(), "unique-local fc00::/7"),
        ("ff00::/8".parse().unwrap(), "multicast ff00::/8"),
    ];
    for (net, label) in blocks {
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
}
