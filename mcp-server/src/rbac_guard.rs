//! Startup guard: the MCP server runs behind rbac-proxy, or not at all.
//!
//! Gateway mode exposes `iii_call` and `iii_observe`, which can reach every
//! function on the engine, so the worker must connect through rbac-proxy for
//! RBAC to decide what MCP clients may do. At boot it calls
//! `rbac-proxy::session`, which the proxy answers itself for every proxied
//! connection and nothing answers for a direct one (`function_not_found`).
//! `III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC=true|1` starts it anyway, loudly.
//!
//! Everything here is pure; `main` performs the call and acts on the
//! [`Decision`].

use serde_json::Value;

/// The escape hatch.
pub const DANGEROUS_ENV: &str = "III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC";

/// The connection-scoped probe rbac-proxy answers itself.
pub const SESSION_PROBE: &str = "rbac-proxy::session";

/// What MCP clients are told when the server runs without RBAC.
pub const DANGEROUS_NOTICE: &str = "WARNING: this MCP server runs WITHOUT RBAC (III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC). Every MCP client can call every function on this engine.";

/// `true` / `1` turn it on; unset or empty leave it off; anything else is
/// refused, so a typo never silently means "unprotected" or "protected".
pub fn parse_dangerous(raw: Option<&str>) -> Result<bool, String> {
    match raw {
        None | Some("") => Ok(false),
        Some("true") | Some("1") => Ok(true),
        Some(other) => Err(format!(
            "mcp-server: {DANGEROUS_ENV}={other} is not a recognised value; use true or 1, or unset it."
        )),
    }
}

#[derive(Debug, Clone)]
pub enum ProbeResult {
    /// The answer to `rbac-proxy::session` (`{ proxied: true, policy, ... }`).
    Proxied(Value),
    /// `function_not_found`, or an answer that does not say `proxied: true`.
    NotProxied,
    /// Anything else: the engine could not be asked.
    Unreachable(String),
}

/// Read the outcome of calling [`SESSION_PROBE`] (the error as text).
pub fn classify_probe(result: Result<Value, String>) -> ProbeResult {
    match result {
        Ok(v) if v.get("proxied").and_then(Value::as_bool) == Some(true) => ProbeResult::Proxied(v),
        Ok(_) => ProbeResult::NotProxied,
        Err(e) if e.contains("function_not_found") || e.contains("FUNCTION_NOT_FOUND") => {
            ProbeResult::NotProxied
        }
        Err(e) => ProbeResult::Unreachable(e),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RbacMode {
    /// Behind rbac-proxy; `policy` is the probe's `policy` object.
    Proxied { policy: Value },
    /// Started with the escape hatch, without RBAC.
    Dangerous,
}

impl RbacMode {
    /// A line for MCP clients (initialize instructions, `iii_status`).
    pub fn client_notice(&self) -> Option<&'static str> {
        match self {
            RbacMode::Dangerous => Some(DANGEROUS_NOTICE),
            RbacMode::Proxied { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Start {
        mode: RbacMode,
        /// Logged at `warn`.
        warnings: Vec<String>,
        /// Logged at `info`.
        notes: Vec<String>,
    },
    Refuse(String),
}

fn not_proxied_message(url: &str) -> String {
    format!(
        "mcp-server: refusing to start: not behind rbac-proxy.
  The MCP server exposes iii_call and iii_observe, which can reach every function
  on this engine. It must connect through rbac-proxy so RBAC decides what MCP
  clients may do.
  Connected to: {url} ({SESSION_PROBE}: function_not_found)
  Fix: install rbac-proxy (iii call compose::add worker=rbac-proxy), set its
  rbac.auth_function_id, and give mcp:
    III_MCP_ENGINE_URL=ws://127.0.0.1:49200   (rbac-proxy's port)
    III_TOKEN=<a token your auth function accepts>
  To run without RBAC anyway (local development only):
    {DANGEROUS_ENV}=true"
    )
}

fn dangerous_warning(url: &str) -> String {
    format!(
        "mcp-server: WARNING: running WITHOUT RBAC ({DANGEROUS_ENV}=true) against {url}.
  Every MCP client can call every function on this engine."
    )
}

/// Whether the probe's `expose_functions` globs include a bare `*`, in any
/// form rbac-proxy reports (`match("*")`, `match('*')`, `{ match: "*" }`).
fn exposes_everything(globs: Option<&Value>) -> bool {
    let Some(Value::Array(rules)) = globs else {
        return false;
    };
    rules.iter().any(|rule| match rule {
        Value::String(s) => matches!(s.trim(), "match(\"*\")" | "match('*')"),
        Value::Object(o) => o.get("match").and_then(Value::as_str) == Some("*"),
        _ => false,
    })
}

pub fn decide(probe: &ProbeResult, dangerous: Result<bool, String>, url: &str) -> Decision {
    let dangerous = match dangerous {
        Ok(d) => d,
        Err(msg) => return Decision::Refuse(msg),
    };
    match probe {
        ProbeResult::Proxied(answer) => {
            let policy = answer.get("policy").cloned().unwrap_or(Value::Null);
            let mut warnings = Vec::new();
            let mut notes = Vec::new();
            let auth = policy.get("auth_function").and_then(Value::as_bool) == Some(true);
            if exposes_everything(policy.get("expose_functions")) && !auth {
                warnings.push(format!(
                    "mcp-server: WARNING: the rbac-proxy at {url} exposes every function and has no auth function; RBAC is not restricting MCP clients."
                ));
            }
            if dangerous {
                notes.push(format!(
                    "mcp-server: {DANGEROUS_ENV} is set but ignored: this connection is behind rbac-proxy."
                ));
            }
            Decision::Start {
                mode: RbacMode::Proxied { policy },
                warnings,
                notes,
            }
        }
        ProbeResult::NotProxied if dangerous => Decision::Start {
            mode: RbacMode::Dangerous,
            warnings: vec![dangerous_warning(url)],
            notes: vec![],
        },
        ProbeResult::NotProxied => Decision::Refuse(not_proxied_message(url)),
        ProbeResult::Unreachable(e) => Decision::Refuse(format!(
            "mcp-server: refusing to start: could not check for rbac-proxy at {url} ({SESSION_PROBE}: {e}). Is the engine (or rbac-proxy) running?"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const URL: &str = "ws://127.0.0.1:49134";

    fn proxied_with(auth: bool, expose_functions: Value) -> ProbeResult {
        ProbeResult::Proxied(json!({
            "proxied": true,
            "proxy_version": "1.1.3",
            "session_id": "s",
            "function_registration_prefix": null,
            "policy": {
                "auth_function": auth,
                "expose_functions": expose_functions,
                "expose_endpoints": ["match(\"/\")"]
            }
        }))
    }

    /// `everything`: the globs include a bare `*`.
    fn proxied(auth: bool, everything: bool) -> ProbeResult {
        let globs = if everything {
            json!(["match(\"*\")"])
        } else {
            json!(["match(\"api::*\")"])
        };
        proxied_with(auth, globs)
    }

    #[test]
    fn the_flag_accepts_only_true_and_1() {
        assert_eq!(parse_dangerous(None), Ok(false));
        assert_eq!(parse_dangerous(Some("")), Ok(false));
        assert_eq!(parse_dangerous(Some("true")), Ok(true));
        assert_eq!(parse_dangerous(Some("1")), Ok(true));
        for bad in ["yes", "TRUE ", "on", "0", "false", "True"] {
            let err = parse_dangerous(Some(bad)).unwrap_err();
            assert!(err.contains(DANGEROUS_ENV), "{bad}: {err}");
            assert!(err.contains("true or 1"), "{bad}: {err}");
        }
    }

    #[test]
    fn an_unrecognised_flag_value_refuses_even_behind_the_proxy() {
        let d = decide(&proxied(true, false), parse_dangerous(Some("yes")), URL);
        let Decision::Refuse(msg) = d else {
            panic!("expected refusal, got {d:?}")
        };
        assert!(msg.contains("not a recognised value"));
    }

    #[test]
    fn function_not_found_means_not_proxied_and_anything_else_is_unreachable() {
        assert!(matches!(
            classify_probe(Err(
                "function_not_found: Function rbac-proxy::session not found".into()
            )),
            ProbeResult::NotProxied
        ));
        assert!(matches!(
            classify_probe(Err("timeout after 5000ms".into())),
            ProbeResult::Unreachable(e) if e.contains("timeout")
        ));
        assert!(matches!(
            classify_probe(Ok(json!({ "proxied": true }))),
            ProbeResult::Proxied(_)
        ));
        // An answer that does not say proxied: true is not proof.
        assert!(matches!(
            classify_probe(Ok(json!({ "ok": true }))),
            ProbeResult::NotProxied
        ));
    }

    #[test]
    fn behind_the_proxy_it_starts() {
        let d = decide(&proxied(true, false), Ok(false), URL);
        let Decision::Start { mode, warnings, .. } = d else {
            panic!("{d:?}")
        };
        assert!(matches!(mode, RbacMode::Proxied { .. }));
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn a_proxy_that_exposes_everything_without_auth_starts_with_a_warning() {
        let Decision::Start { warnings, .. } = decide(&proxied(false, true), Ok(false), URL) else {
            panic!()
        };
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("exposes every function"));
    }

    #[test]
    fn the_everything_warning_reads_the_globs_in_every_form() {
        for globs in [
            json!(["match(\"api::*\")", "match(\"*\")"]),
            json!(["match('*')"]),
            json!([{ "match": "*" }]),
            json!([{ "match": "*", "namespace": "orders" }]),
        ] {
            let Decision::Start { warnings, .. } =
                decide(&proxied_with(false, globs.clone()), Ok(false), URL)
            else {
                panic!()
            };
            assert_eq!(warnings.len(), 1, "{globs}");
        }
        for globs in [
            json!(["match(\"api::*\")"]),
            json!([{ "match": "*::public" }]),
            json!([{ "metadata": { "public": true } }]),
            json!([]),
            json!(3),
        ] {
            let Decision::Start { warnings, .. } =
                decide(&proxied_with(false, globs.clone()), Ok(false), URL)
            else {
                panic!()
            };
            assert!(warnings.is_empty(), "{globs}: {warnings:?}");
        }
        // With an auth function, `*` is the operator's choice: no warning.
        let Decision::Start { warnings, .. } = decide(&proxied(true, true), Ok(false), URL) else {
            panic!()
        };
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn the_flag_is_ignored_with_an_info_note_behind_the_proxy() {
        let Decision::Start {
            mode,
            warnings,
            notes,
        } = decide(&proxied(true, false), Ok(true), URL)
        else {
            panic!()
        };
        assert!(matches!(mode, RbacMode::Proxied { .. }));
        assert!(warnings.is_empty(), "not a warning: {warnings:?}");
        assert!(notes.iter().any(|n| n.contains("ignored")), "{notes:?}");
    }

    #[test]
    fn not_proxied_refuses_with_the_fix_and_the_flag() {
        let Decision::Refuse(msg) = decide(&ProbeResult::NotProxied, Ok(false), URL) else {
            panic!()
        };
        assert!(
            msg.starts_with("mcp-server: refusing to start: not behind rbac-proxy."),
            "{msg}"
        );
        assert!(msg.contains(URL));
        assert!(msg.contains("compose::add worker=rbac-proxy"));
        assert!(
            msg.contains("III_MCP_ENGINE_URL=ws://127.0.0.1:49200"),
            "{msg}"
        );
        assert!(msg.contains("III_MCP_DANGEROUSLY_RUN_WITHOUT_RBAC=true"));
    }

    #[test]
    fn not_proxied_with_the_flag_starts_dangerously_and_says_so() {
        let Decision::Start { mode, warnings, .. } =
            decide(&ProbeResult::NotProxied, Ok(true), URL)
        else {
            panic!()
        };
        assert!(matches!(mode, RbacMode::Dangerous));
        assert!(
            warnings[0].starts_with("mcp-server: WARNING: running WITHOUT RBAC"),
            "{warnings:?}"
        );
        assert!(warnings[0].contains("Every MCP client can call every function"));
        assert!(DANGEROUS_NOTICE.contains("Every MCP client can call every function"));
    }

    #[test]
    fn an_unreachable_engine_refuses_whatever_the_flag() {
        for flag in [Ok(false), Ok(true)] {
            let d = decide(
                &ProbeResult::Unreachable("connection refused".into()),
                flag,
                URL,
            );
            let Decision::Refuse(msg) = d else {
                panic!("{d:?}")
            };
            assert!(msg.contains("could not check"), "{msg}");
            assert!(msg.contains("connection refused"));
        }
    }

    #[test]
    fn the_mode_tells_clients_when_rbac_is_off() {
        assert_eq!(RbacMode::Dangerous.client_notice(), Some(DANGEROUS_NOTICE));
        assert_eq!(
            RbacMode::Proxied { policy: json!({}) }.client_notice(),
            None
        );
    }
}
