//! Bridge ingress reconciliation. Two inbound transports, picked by config:
//!
//! - **Socket Mode** (`app_token` set): the worker dials out to Slack over a
//!   WebSocket. No public URL needed. Takes precedence.
//! - **HTTP / Events API** (`signing_secret` + `public_base_url`): register the
//!   `slack/events` and `slack/interactions` routes on the engine.
//!
//! Driven at boot and on every config reload.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::trigger::Trigger;
use iii_sdk::IIIClient;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::clients::socket;
use crate::config::{EVENTS_API_PATH, INTERACTIONS_API_PATH};
use crate::deps::Deps;
use crate::functions::{events::EVENTS_ID, interactions::INTERACTIONS_ID};

/// Reconcile both inbound transports to the current config, and refresh the
/// cached Slack identity (the bot user id the bridge needs for mention handling).
pub async fn apply(deps: &Arc<Deps>) {
    if let Ok(id) = crate::clients::slack::auth_test(deps).await {
        *deps.identity.write().await = Some(id);
    }
    let cfg = deps.cfg().await;
    reconcile_socket(deps, cfg.socket_enabled()).await;
    reconcile_http(deps, cfg.http_bridge_enabled()).await;
}

async fn reconcile_socket(deps: &Arc<Deps>, enabled: bool) {
    let mut guard = deps.socket_cancel.lock().await;
    match (enabled, guard.is_some()) {
        (true, false) => {
            let cancel = CancellationToken::new();
            let child = cancel.clone();
            let deps = deps.clone();
            tokio::spawn(async move { socket::run(deps, child).await });
            *guard = Some(cancel);
            tracing::info!("socket mode enabled (no public URL required)");
        }
        (false, true) => {
            if let Some(cancel) = guard.take() {
                cancel.cancel();
            }
            tracing::info!("socket mode disabled");
        }
        _ => {}
    }
}

async fn reconcile_http(deps: &Arc<Deps>, enabled: bool) {
    let mut guard = deps.bridge_triggers.lock().await;
    if enabled && guard.is_empty() {
        for (id, path) in [
            (EVENTS_ID, EVENTS_API_PATH),
            (INTERACTIONS_ID, INTERACTIONS_API_PATH),
        ] {
            match register_route(&deps.iii, id, path) {
                Ok(t) => {
                    guard.push(t);
                    tracing::info!(function_id = id, api_path = path, "bridge route registered");
                }
                Err(e) => {
                    tracing::error!(error = %e, function_id = id, "failed to register bridge route")
                }
            }
        }
        if let Some(url) = deps.cfg().await.events_url() {
            tracing::info!(events_url = %url, "http bridge enabled — point Slack Event Subscriptions here");
        }
    } else if !enabled && !guard.is_empty() {
        for t in guard.drain(..) {
            t.unregister();
        }
        tracing::info!("http bridge disabled — routes removed");
    }
}

/// Stop both transports on shutdown.
pub async fn shutdown(deps: &Arc<Deps>) {
    if let Some(cancel) = deps.socket_cancel.lock().await.take() {
        cancel.cancel();
    }
    let mut guard = deps.bridge_triggers.lock().await;
    for t in guard.drain(..) {
        t.unregister();
    }
}

fn register_route(iii: &IIIClient, function_id: &str, api_path: &str) -> Result<Trigger, Error> {
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: function_id.to_string(),
        config: json!({ "api_path": api_path, "http_method": "POST" }),
        metadata: None,
    })
}
