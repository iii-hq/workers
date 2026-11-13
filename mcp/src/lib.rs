//! `mcp` — MCP 2025-06-18 Streamable HTTP bridge for the iii engine.
//!
//! The binary in [`main`](../mcp/index.html) is a thin wrapper that wires
//! the modules below to the iii engine. Tests under `tests/` reach into
//! these modules to register the bridge in-process against a shared
//! engine handle (the same code path the production binary takes).
//!
//! Surface:
//!
//!   * [`config`]: [`config::McpConfig`] and the engine URL precedence.
//!   * [`functions`]: registers `mcp::handler` (the endpoint, not
//!     observable) and `mcp::observed` (the observer sink), and observes
//!     `http::on-request` for the endpoint's routes.
//!   * [`handler`]: JSON-RPC dispatch for `initialize`, `tools/*`,
//!     `resources/*`.
//!   * [`gateway`]: the gateway tools (`iii_call`, `iii_secret_store`,
//!     `iii_observe`, ...).
//!   * [`directory_bridge`]: agent profiles and skills from iii-directory.
//!   * [`rbac_guard`]: refuses to start unless behind rbac-proxy.
//!   * [`sessions`]: Streamable HTTP sessions and their observers.
//!   * [`manifest`]: the `--manifest` payload.

pub mod bus;
pub mod config;
pub mod directory_bridge;
pub mod engine;
pub mod functions;
pub mod gateway;
pub mod handler;
pub mod http_guard;
pub mod jsonrpc;
pub mod manifest;
pub mod origin;
pub mod rbac_guard;
pub mod sessions;
pub mod sink_guard;
pub mod transport;

#[cfg(test)]
pub mod testkit;

/// Keep request and response bodies out of this worker's trace spans: a
/// tool call can carry a credential on its way to `iii_secret_store`. Call
/// it before connecting to the engine.
pub fn disable_trace_payloads() {
    std::env::set_var("III_DISABLE_TRACE_PAYLOADS", "1");
}

#[cfg(test)]
mod startup_tests {
    #[test]
    fn trace_payloads_are_turned_off() {
        // A tool call can carry a credential for iii_secret_store.
        std::env::remove_var("III_DISABLE_TRACE_PAYLOADS");
        super::disable_trace_payloads();
        assert_eq!(
            std::env::var("III_DISABLE_TRACE_PAYLOADS").as_deref(),
            Ok("1")
        );
    }

    #[test]
    fn the_worker_declares_its_dependencies() {
        let yaml: serde_yaml::Value =
            serde_yaml::from_str(include_str!("../iii.worker.yaml")).unwrap();
        let deps = yaml["dependencies"].as_mapping().expect("dependencies");
        for dep in ["http", "rbac-proxy", "secrets"] {
            assert!(
                deps.contains_key(serde_yaml::Value::String(dep.into())),
                "{dep} missing"
            );
        }
    }
}
