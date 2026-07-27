//! Connect-or-skip wrapper around the iii SDK plus in-process
//! registration of `mcp::handler` and its HTTP trigger.
//!
//! Lifted from
//! [`skills/tests/common/engine.rs`](../../../../skills/tests/common/engine.rs)
//! and adapted: after the engine probe succeeds, register the same
//! function + trigger pair the production binary registers in
//! [`mcp/src/main.rs`](../../../src/main.rs) so HTTP scenarios can post
//! to the engine's `/mcp` endpoint and reach our in-process handler.
//!
//! One engine connection per test binary process via [`OnceCell`] —
//! the cost of each WebSocket handshake dwarfs cucumber's per-scenario
//! overhead.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::{
    errors::Error,
    protocol::{RegisterTriggerInput, TriggerRequest},
    register_worker, IIIClient, InitOptions,
};
use serde_json::json;
use tokio::net::TcpStream;
use tokio::sync::OnceCell;

use iii_mcp::{
    config::McpConfig,
    functions::{self, FUNCTION_ID},
};

const DEFAULT_WS_URL: &str = "ws://127.0.0.1:49134";
const DEFAULT_HTTP_URL: &str = "http://127.0.0.1:3000/mcp";

static ENGINE: OnceCell<Option<Arc<IIIClient>>> = OnceCell::const_new();

pub fn ws_url() -> String {
    std::env::var("III_ENGINE_WS_URL").unwrap_or_else(|_| DEFAULT_WS_URL.to_string())
}

pub fn http_url() -> String {
    std::env::var("III_ENGINE_HTTP_URL").unwrap_or_else(|_| DEFAULT_HTTP_URL.to_string())
}

/// Cheap reachability check on the engine HTTP server. We only need
/// the host:port to accept TCP connections — the trigger we just
/// registered may take a beat to start routing, so we don't actually
/// post anything yet. Returns the probed `host:port` on success.
async fn http_reachable(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let port = parsed.port_or_known_default()?;
    let target = format!("{host}:{port}");
    TcpStream::connect(&target).await.ok()?;
    Some(target)
}

/// Probe the engine via the cheapest call we know of (`engine::workers::list`).
/// Retries for ~5s while the WebSocket completes the handshake. Returns
/// `None` and prints a single skip notice when the engine is unreachable.
pub async fn try_connect_raw() -> Option<Arc<IIIClient>> {
    let url = ws_url();
    let iii = Arc::new(register_worker(&url, InitOptions::default()));

    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let probe = iii
            .trigger(TriggerRequest {
                function_id: "engine::workers::list".to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(800),
            })
            .await;
        match probe {
            Ok(_) => return Some(iii),
            Err(Error::NotConnected) => continue,
            Err(_) => continue,
        }
    }

    eprintln!(
        "[skip] iii engine not reachable at {url}; \
         set III_ENGINE_WS_URL or start `iii` to enable engine-bound BDD scenarios"
    );
    iii.shutdown_async().await;
    None
}

/// Register `mcp::handler` and the HTTP trigger at `cfg.api_path`.
/// Mirrors the boot sequence in
/// [`mcp/src/main.rs`](../../../src/main.rs). Registration errors are
/// logged but not propagated so a polluted engine (e.g. a stale `mcp`
/// binary still running) degrades to a soft skip rather than a hard
/// failure inside cucumber's `before` hook.
async fn register_bridge(iii: &Arc<IIIClient>) {
    let cfg = Arc::new(McpConfig::default());
    functions::register_all(iii, &cfg);

    let api_path = cfg.api_path.clone();
    if let Err(e) = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: FUNCTION_ID.to_string(),
        config: json!({
            "api_path": api_path,
            "http_method": "POST",
        }),
        metadata: None,
        namespace: iii.namespace(),
    }) {
        eprintln!(
            "[warn] mcp BDD harness failed to register HTTP trigger at {api_path}: {e}; \
             POST /mcp scenarios may fall through to whatever (if anything) is already \
             registered against this engine"
        );
    }

    // Give the engine a beat to publish the registration before
    // scenarios start posting to /mcp.
    tokio::time::sleep(Duration::from_millis(150)).await;
}

/// Get-or-init the shared engine handle. Subsequent callers in the
/// same binary reuse the connection. Soft-skips by returning `None`
/// when:
///
///   * the WebSocket isn't reachable (no `iii` running, or running on
///     a non-default WS URL with no `III_ENGINE_WS_URL` override);
///   * or the engine HTTP server isn't reachable on `III_ENGINE_HTTP_URL`
///     (default `http://127.0.0.1:3000/mcp`). The bridge always speaks
///     HTTP, so an `iii` whose HTTP listener is disabled or on a
///     different port can't drive the `@engine` scenarios either.
pub async fn get_or_init() -> Option<Arc<IIIClient>> {
    ENGINE
        .get_or_init(|| async {
            let iii = try_connect_raw().await?;
            register_bridge(&iii).await;

            let url = http_url();
            if http_reachable(&url).await.is_none() {
                eprintln!(
                    "[skip] iii engine HTTP not reachable at {url}; \
                     set III_ENGINE_HTTP_URL or start `iii` with HTTP enabled to \
                     run engine-bound BDD scenarios"
                );
                iii.shutdown_async().await;
                return None;
            }

            Some(iii)
        })
        .await
        .clone()
}
