//! Where the MCP endpoint is reachable from: the http worker's bind address.
//!
//! `mcp-server` serves its endpoint through the http worker, whose default host is
//! `0.0.0.0` (every interface). An MCP endpoint reachable from the network
//! lets anyone who can reach it use the gateway's tools, so at boot `mcp-server`
//! reads `http::status` and refuses to serve on anything but loopback unless
//! `III_MCP_DANGEROUSLY_ALLOW_PUBLIC_HTTP=true|1`. Several mcp-server workers share
//! one http worker on different `api_path`s.

use std::future::Future;
use std::net::IpAddr;
use std::time::Duration;

use serde_json::Value;

/// After a check that could not read `http::status`, the waits before each
/// recovery check. The routes stay withdrawn meanwhile (the bind is
/// unknown); after the last one, the next reload of the http worker checks.
pub const RECOVERY_DELAYS: [Duration; 5] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
    Duration::from_secs(60),
];

/// Whether `http::status` left the bind unknown (an error, or no host).
/// A public bind is known: it is refused, not retried.
pub fn status_unknown(status: &Result<Value, String>) -> bool {
    !matches!(status, Ok(s) if s.get("host").and_then(Value::as_str).is_some())
}

/// Run `check` after each of `delays` until one reads the bind (`true`), or
/// `current` says a newer check took over. Returns how many checks ran.
pub async fn recover<C, F, Fut>(delays: &[Duration], current: C, mut check: F) -> usize
where
    C: Fn() -> bool,
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let mut tried = 0;
    for delay in delays {
        tokio::time::sleep(*delay).await;
        if !current() {
            break;
        }
        tried += 1;
        if check().await {
            break;
        }
    }
    tried
}

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

/// What to do with the routes when the http worker's bind is re-checked
/// after a reload.
#[derive(Debug, Clone, PartialEq)]
pub enum RouteAction {
    Keep,
    /// Stop serving: the endpoint would be reachable where it must not be.
    Withdraw(String),
    /// Serve again: the bind is acceptable once more.
    Restore,
}

pub fn transition(serving: bool, bind: &Bind) -> RouteAction {
    match (serving, bind) {
        (true, Bind::Refuse(why)) => RouteAction::Withdraw(why.clone()),
        (false, Bind::Serve { .. }) => RouteAction::Restore,
        _ => RouteAction::Keep,
    }
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
    let unknown = |why: String| {
        if allow_public {
            Bind::Serve {
                warnings: vec![format!(
                    "mcp-server: WARNING: {why}; serving {api_path} anyway ({PUBLIC_HTTP_ENV}=true)."
                )],
            }
        } else {
            Bind::Refuse(format!(
                "mcp-server: refusing to serve {api_path}: {why}, so it may be reachable from other machines.
  Fix: let mcp-server read http::status (rbac-proxy must expose it), with the http
  worker bound to 127.0.0.1. To serve without knowing:
    {PUBLIC_HTTP_ENV}=true"
            ))
        }
    };
    let host = match status
        .as_ref()
        .map(|s| s.get("host").and_then(Value::as_str))
    {
        Ok(Some(host)) => host.to_string(),
        Ok(None) => return unknown("http::status did not report a host".into()),
        Err(e) => return unknown(format!("could not read http::status ({e})")),
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

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    const QUICK: [Duration; 5] = [Duration::from_millis(1); 5];

    #[test]
    fn recovery_waits_2_5_10_30_then_60_seconds() {
        let secs: Vec<u64> = RECOVERY_DELAYS.iter().map(|d| d.as_secs()).collect();
        assert_eq!(secs, vec![2, 5, 10, 30, 60]);
    }

    #[test]
    fn only_an_unreadable_status_is_unknown() {
        assert!(status_unknown(&Err("timed out".into())));
        assert!(status_unknown(&Ok(json!({ "port": 3111 }))));
        assert!(!status_unknown(&status("127.0.0.1")));
        // A public bind is known: refused, not retried.
        assert!(!status_unknown(&status("0.0.0.0")));
    }

    #[tokio::test]
    async fn recovery_stops_at_the_first_check_that_reads_the_bind() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let tried = recover(
            &QUICK,
            || true,
            move || {
                let n = c.fetch_add(1, Ordering::SeqCst) + 1;
                async move { n >= 3 }
            },
        )
        .await;
        assert_eq!(tried, 3);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn recovery_gives_up_after_the_last_delay() {
        let tried = recover(&QUICK, || true, || async { false }).await;
        assert_eq!(tried, 5, "then the next reload re-checks");
    }

    #[tokio::test]
    async fn a_newer_check_cancels_an_older_recovery() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (c, seen) = (calls.clone(), calls.clone());
        let tried = recover(
            &QUICK,
            move || seen.load(Ordering::SeqCst) < 2,
            move || {
                c.fetch_add(1, Ordering::SeqCst);
                async { false }
            },
        )
        .await;
        assert_eq!(tried, 2);
    }

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
    fn an_unknown_bind_is_refused_unless_the_flag_is_set() {
        // No host, or no answer at all: loopback is unconfirmed.
        for status in [
            Ok(json!({ "port": 1 })),
            Err("function_not_found".to_string()),
        ] {
            let Bind::Refuse(msg) = decide(status.clone(), Ok(false), "/mcp") else {
                panic!("{status:?}")
            };
            assert!(
                msg.contains("http::status") && msg.contains(PUBLIC_HTTP_ENV),
                "{msg}"
            );
            let Bind::Serve { warnings } = decide(status, Ok(true), "/mcp") else {
                panic!()
            };
            assert_eq!(warnings.len(), 1);
        }
        assert!(matches!(
            decide(status("127.0.0.1"), parse_flag(Some("on")), "/mcp"),
            Bind::Refuse(_)
        ));
    }

    #[test]
    fn a_reload_withdraws_or_restores_the_routes() {
        let serve = Bind::Serve { warnings: vec![] };
        let refuse = Bind::Refuse("public".into());
        assert_eq!(transition(true, &serve), RouteAction::Keep);
        assert_eq!(transition(false, &refuse), RouteAction::Keep);
        assert_eq!(
            transition(true, &refuse),
            RouteAction::Withdraw("public".into())
        );
        assert_eq!(transition(false, &serve), RouteAction::Restore);
    }
}
