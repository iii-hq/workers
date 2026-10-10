//! `console::status` — health/identity probe.
//!
//! Returns the runtime knobs the worker is operating under so callers
//! (operators, agents, `iii worker info` smoke tests, dashboards) can
//! confirm the worker is alive and tell where the UI is without parsing
//! logs or assuming port 3113.

use std::net::SocketAddr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct StatusInput {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusOutput {
    /// TCP port the worker is serving the UI and `/ws` on.
    pub http_port: u16,
    /// Base URL to open the UI from this machine (e.g.
    /// `http://127.0.0.1:3113`), from the bound host and the live port.
    pub url: String,
    /// iii engine WebSocket URL the worker is proxying to.
    pub engine_url: String,
    /// Worker version (matches `Cargo.toml`).
    pub version: String,
}

/// The base URL a local client opens for a listener bound to `addr` — the
/// same rule as `http::status`: an every-interface bind opens on loopback.
pub fn local_url(addr: SocketAddr) -> String {
    let host = match addr.ip() {
        ip if ip.is_unspecified() && ip.is_ipv4() => "127.0.0.1".to_string(),
        ip if ip.is_unspecified() => "[::1]".to_string(),
        std::net::IpAddr::V6(ip) => format!("[{ip}]"),
        ip => ip.to_string(),
    };
    format!("http://{host}:{}", addr.port())
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_url_uses_loopback_for_every_interface() {
        assert_eq!(
            super::local_url("0.0.0.0:3213".parse().unwrap()),
            "http://127.0.0.1:3213"
        );
        assert_eq!(
            super::local_url("127.0.0.1:3113".parse().unwrap()),
            "http://127.0.0.1:3113"
        );
        assert_eq!(
            super::local_url("[::]:3113".parse().unwrap()),
            "http://[::1]:3113"
        );
        assert_eq!(
            super::local_url("192.168.0.2:3113".parse().unwrap()),
            "http://192.168.0.2:3113"
        );
    }
}
