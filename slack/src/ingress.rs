//! Bridge ingress: register the Slack events + interactions HTTP routes on the
//! engine when the bridge is enabled (`public_base_url` + `signing_secret`),
//! and remove them when it is disabled. Driven at boot and on config reload.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::trigger::Trigger;
use iii_sdk::IIIClient;
use serde_json::json;

use crate::config::{EVENTS_API_PATH, INTERACTIONS_API_PATH};
use crate::deps::Deps;
use crate::functions::{events::EVENTS_ID, interactions::INTERACTIONS_ID};

/// Reconcile the HTTP routes to the current config.
pub async fn apply(deps: &Arc<Deps>) {
    let enabled = deps.cfg().await.bridge_enabled();
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
            tracing::info!(events_url = %url, "bridge enabled — point Slack Event Subscriptions here");
        }
    } else if !enabled && !guard.is_empty() {
        for t in guard.drain(..) {
            t.unregister();
        }
        tracing::info!("bridge disabled — routes removed");
    }
}

/// Remove routes on shutdown.
pub async fn shutdown(deps: &Arc<Deps>) {
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
