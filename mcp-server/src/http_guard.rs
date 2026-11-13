//! Where the MCP endpoint is reachable from: the http worker's bind address.
//!
//! `mcp-server` serves its endpoint through the http worker, whose default host is
//! `0.0.0.0` (every interface). An MCP endpoint reachable from the network
//! lets anyone who can reach it use the gateway's tools, so at boot `mcp-server`
//! reads `http::status` and refuses to serve on anything but loopback unless
//! `III_MCP_DANGEROUSLY_ALLOW_PUBLIC_HTTP=true|1`. Several mcp-server workers share
//! one http worker on different `api_path`s.

use std::net::IpAddr;

use serde_json::Value;

/// The escape hatch for a non-loopback http worker.
pub const PUBLIC_HTTP_ENV: &str = "III_MCP_DANGEROUSLY_ALLOW_PUBLIC_HTTP";

/// `true` / `1` turn it on; unset or empty leave it off; anything else is
/// refused, so a typo never silently changes the exposure.
pub fn parse_flag(raw: Option<&str>) -> Result<bool, String> {
    match raw {
        None | Some("") => Ok(false),
        Some("true") | Some("1") => Ok(true),
        Some(other) => Err(format!(
            "mcp-server: {PUBLIC_HTTP_ENV}={other} is not a recognised value; use true or 1, or unset it."
        )),
    }
}

/// Loopback only: `127.0.0.0/8`, `::1`, `localhost`. Unspecified (`0.0.0.0`,
/// `::`), private, public and multicast addresses are all reachable from
/// somewhere else.
pub fn is_loopback(host: &str) -> bool {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[derive(Debug, Clone, PartialEq)]
pub enum Bind {
    Serve { warnings: Vec<String> },
    Refuse(String),
}

/// Decide from the answer to `http::status` (the error as text) and the flag.
pub fn decide(status: Result<Value, String>, flag: Result<bool, String>, api_path: &str) -> Bind {
    let allow_public = match flag {
        Ok(f) => f,
        Err(msg) => return Bind::Refuse(msg),
    };
    let host = match status.as_ref().map(|s| s.get("host").and_then(Value::as_str)) {
        Ok(Some(host)) => host.to_string(),
        Ok(None) => {
            return Bind::Serve {
                warnings: vec![format!(
                    "mcp-server: WARNING: http::status did not report a host; cannot tell whether {api_path} is reachable from the network."
                )],
            }
        }
        Err(e) => {
            return Bind::Serve {
                warnings: vec![format!(
                    "mcp-server: WARNING: could not read http::status ({e}); cannot tell whether {api_path} is reachable from the network."
                )],
            }
        }
    };
    if is_loopback(&host) {
        return Bind::Serve { warnings: vec![] };
    }
    if allow_public {
        return Bind::Serve {
            warnings: vec![format!(
                "mcp-server: WARNING: serving {api_path} on an http worker bound to {host} ({PUBLIC_HTTP_ENV}=true). Anyone who can reach it can use every MCP tool, within what rbac-proxy allows."
            )],
        };
    }
    Bind::Refuse(format!(
        "mcp-server: refusing to serve {api_path}: the http worker listens on {host}, which is reachable from other machines.
  Fix: bind the http worker to loopback. Its configuration entry id is the answer to
  http::configuration-id; set \"host\": \"127.0.0.1\" there with configuration::set.
  To serve on {host} anyway (behind your own firewall or proxy):
    {PUBLIC_HTTP_ENV}=true"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn status(host: &str) -> Result<Value, String> {
        Ok(json!({ "host": host, "port": 3111, "url": "http://127.0.0.1:3111" }))
    }

    #[test]
    fn loopback_hosts_are_local_and_everything_else_is_public() {
        for h in [
            "127.0.0.1",
            "127.0.0.53",
            "::1",
            "[::1]",
            "localhost",
            "LOCALHOST",
        ] {
            assert!(is_loopback(h), "{h}");
        }
        for h in [
            "0.0.0.0",
            "::",
            "[::]",
            "192.168.1.10",
            "10.0.0.1",
            "224.0.0.1",
            "ff02::1",
            "example.com",
            "",
        ] {
            assert!(!is_loopback(h), "{h}");
        }
    }

    #[test]
    fn the_flag_accepts_only_true_and_1() {
        assert_eq!(parse_flag(None), Ok(false));
        assert_eq!(parse_flag(Some("")), Ok(false));
        assert_eq!(parse_flag(Some("true")), Ok(true));
        assert_eq!(parse_flag(Some("1")), Ok(true));
        let err = parse_flag(Some("yes")).unwrap_err();
        assert!(
            err.contains(PUBLIC_HTTP_ENV) && err.contains("true or 1"),
            "{err}"
        );
    }

    #[test]
    fn a_loopback_bind_serves_quietly() {
        assert_eq!(
            decide(status("127.0.0.1"), Ok(false), "/mcp"),
            Bind::Serve { warnings: vec![] }
        );
    }

    #[test]
    fn a_public_bind_is_refused_with_the_fix() {
        for host in ["0.0.0.0", "192.168.1.10", "::"] {
            let Bind::Refuse(msg) = decide(status(host), Ok(false), "/mcp") else {
                panic!("{host}")
            };
            assert!(msg.contains(host), "{msg}");
            assert!(msg.contains("127.0.0.1"), "{msg}");
            assert!(msg.contains(PUBLIC_HTTP_ENV), "{msg}");
        }
    }

    #[test]
    fn the_flag_serves_a_public_bind_with_a_warning() {
        let Bind::Serve { warnings } = decide(status("0.0.0.0"), Ok(true), "/mcp") else {
            panic!()
        };
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("0.0.0.0") && warnings[0].contains("WARNING"),
            "{warnings:?}"
        );
        // On loopback the flag changes nothing.
        assert_eq!(
            decide(status("127.0.0.1"), Ok(true), "/mcp"),
            Bind::Serve { warnings: vec![] }
        );
    }

    #[test]
    fn an_unreadable_status_warns_and_a_bad_flag_refuses() {
        let Bind::Serve { warnings } = decide(Err("function_not_found".into()), Ok(false), "/mcp")
        else {
            panic!()
        };
        assert!(warnings[0].contains("http::status"), "{warnings:?}");
        assert!(matches!(
            decide(status("127.0.0.1"), parse_flag(Some("on")), "/mcp"),
            Bind::Refuse(_)
        ));
        let Bind::Serve { warnings } = decide(Ok(json!({ "port": 1 })), Ok(false), "/mcp") else {
            panic!()
        };
        assert_eq!(warnings.len(), 1);
    }
}
