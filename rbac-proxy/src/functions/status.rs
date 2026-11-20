//! `rbac-proxy::status` — health/identity probe for `engine::workers::info` smoke
//! tests and dashboards. Read-only operational metadata; sync invocation.
//!
//! The `engine_url` is passed through [`crate::redact_url`] before it leaves
//! the worker, so a credentialed `wss://user:secret@host` upstream never leaks
//! to an agent-callable probe.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;
use crate::rbac::AuthMode;

pub const STATUS_ID: &str = "rbac-proxy::status";
pub const STATUS_DESC: &str = "Check proxy health: bound host/port, the (credential-redacted) upstream engine URL, which authentication admits worker and telemetry connections (auth function, loopback grant, or none; channel sockets are checked by the engine), which endpoints are exposed, and the live downstream connection count.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct StatusInput {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusOutput {
    /// Bound public host.
    pub host: String,
    /// Bound public RBAC port.
    pub port: u16,
    /// Upstream engine URL, with any `user:pass@` credentials redacted.
    pub engine_url: String,
    /// Whether an `auth_function_id` is configured.
    pub rbac_enabled: bool,
    /// The authentication in force for new worker (`/`) and telemetry
    /// (`/otel`) connections: `function` (the `auth_function_id`),
    /// `loopback` (no auth function; `loopback_auth` admits loopback peers)
    /// or `none` (every worker and telemetry connection is refused).
    /// `/ws/channels/*` sockets are not authenticated by the proxy in any
    /// mode: the engine checks their channel access keys.
    pub auth: AuthMode,
    /// The `rbac.expose_endpoints` patterns: the paths the proxy serves.
    /// Empty: every connection is refused.
    pub expose_endpoints: Vec<String>,
    /// Live downstream connections.
    pub active_connections: u32,
    /// Worker version (matches `Cargo.toml`).
    pub version: String,
}

/// The status of a proxy running `cfg`, bound to `host:port`, with `active`
/// live connections.
pub fn status_output(cfg: &WorkerConfig, host: String, port: u16, active: u32) -> StatusOutput {
    StatusOutput {
        host,
        port,
        engine_url: crate::redact_url(&cfg.engine_url),
        rbac_enabled: cfg.rbac_enabled(),
        auth: cfg.rbac.auth_mode(),
        expose_endpoints: cfg
            .rbac
            .endpoints()
            .iter()
            .map(|rule| rule.pattern().to_string())
            .collect(),
        active_connections: active,
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rbac::LoopbackAuth;
    use serde_json::json;

    fn status_of(cfg: &WorkerConfig) -> serde_json::Value {
        serde_json::to_value(status_output(cfg, "127.0.0.1".into(), 49200, 3)).unwrap()
    }

    #[test]
    fn status_reports_the_auth_in_force() {
        let mut cfg = WorkerConfig::default();
        let none = status_of(&cfg);
        assert_eq!(none["auth"], "none");
        assert_eq!(none["rbac_enabled"], false);

        cfg.rbac.loopback_auth = Some(LoopbackAuth::default());
        let loopback = status_of(&cfg);
        assert_eq!(loopback["auth"], "loopback");
        // rbac_enabled keeps its meaning: an auth function is configured.
        assert_eq!(loopback["rbac_enabled"], false);

        cfg.rbac.auth_function_id = Some("p::auth".into());
        let function = status_of(&cfg);
        assert_eq!(function["auth"], "function");
        assert_eq!(function["rbac_enabled"], true);

        cfg.rbac.loopback_auth = None;
        assert_eq!(status_of(&cfg)["auth"], "function");
        assert_eq!(
            status_output(&cfg, "h".into(), 1, 0).auth,
            AuthMode::Function
        );
    }

    #[test]
    fn the_auth_mode_docs_scope_it_to_the_worker_and_telemetry_endpoints() {
        // Channel sockets bypass the proxy's authentication: the engine
        // checks their access keys. Every description of `auth` says so.
        let schema = serde_json::to_value(schemars::schema_for!(StatusOutput)).unwrap();
        let mut docs = vec![schema["properties"]["auth"]["description"].clone()];
        docs.push(schema["definitions"]["AuthMode"]["description"].clone());
        for variant in schema["definitions"]["AuthMode"]["oneOf"]
            .as_array()
            .unwrap()
        {
            docs.push(variant["description"].clone());
        }
        let scoped = |d: &str| d.contains("`/`") && d.contains("`/otel`");
        let top = docs[0].as_str().unwrap();
        assert!(scoped(top) && top.contains("/ws/channels/*"), "{top}");
        let mode = docs[1].as_str().unwrap();
        assert!(scoped(mode) && mode.contains("/ws/channels/*"), "{mode}");
        let none = docs[4].as_str().unwrap();
        assert!(
            none.contains("/ws/channels/*") && none.contains("not"),
            "{none}"
        );
    }

    #[test]
    fn status_keeps_its_existing_fields() {
        let mut cfg = WorkerConfig {
            engine_url: "ws://user:secret@engine:49134".into(),
            ..WorkerConfig::default()
        };
        cfg.rbac.loopback_auth = Some(LoopbackAuth {
            worker_names: vec!["mcp-server".into()],
            ..LoopbackAuth::default()
        });
        let s = status_of(&cfg);
        assert_eq!(s["host"], "127.0.0.1");
        assert_eq!(s["port"], 49200);
        assert!(!s["engine_url"].as_str().unwrap().contains("secret"), "{s}");
        assert_eq!(s["expose_endpoints"], json!(["/"]));
        assert_eq!(s["active_connections"], 3);
        assert_eq!(s["version"], env!("CARGO_PKG_VERSION"));
        // The grants themselves are not reported.
        assert!(!s.to_string().contains("mcp-server"), "{s}");
        let mut keys: Vec<_> = s.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "active_connections",
                "auth",
                "engine_url",
                "expose_endpoints",
                "host",
                "port",
                "rbac_enabled",
                "version"
            ]
        );
    }
}
