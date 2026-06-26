pub mod bindings;
pub mod notify;
pub mod set_webhook;
pub mod webhook;

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::trigger::Trigger;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::json;

use crate::deps::Deps;

pub const WEBHOOK_ID: &str = "telegram-bot::webhook";
pub const SET_WEBHOOK_ID: &str = "telegram-bot::set-webhook";
pub const NOTIFY_ID: &str = "telegram-bot::notify";
pub const ON_MESSAGE_ADDED_ID: &str = "telegram-bot::on-message-added";
pub const ON_MESSAGE_UPDATED_ID: &str = "telegram-bot::on-message-updated";
pub const ON_STATUS_CHANGED_ID: &str = "telegram-bot::on-status-changed";
pub const ON_TURN_COMPLETED_ID: &str = "telegram-bot::on-turn-completed";
pub const ON_PENDING_CREATED_ID: &str = "telegram-bot::on-pending-created";
pub const ON_PENDING_RESOLVED_ID: &str = "telegram-bot::on-pending-resolved";

fn register<Req, Resp, F, Fut>(
    iii: &Arc<IIIClient>,
    deps: &Arc<Deps>,
    id: &str,
    description: &str,
    handler: F,
) where
    Req: DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Deps>, Req) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<Resp, Error>> + Send + 'static,
{
    let deps = deps.clone();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |req: Req| {
            let deps = deps.clone();
            let handler = handler.clone();
            async move { handler(deps, req).await }
        })
        .description(description),
    );
}

pub fn register_all(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    webhook::register(iii, deps);
    set_webhook::register(iii, deps);
    notify::register(iii, deps);
    bindings::register(iii, deps);
    tracing::info!("all functions registered");
}

pub fn bind_triggers(iii: &Arc<IIIClient>) {
    let bindings = [
        (
            "session::message-added",
            ON_MESSAGE_ADDED_ID,
            json!({ "roles": ["assistant", "function_result"] }),
        ),
        (
            "session::message-updated",
            ON_MESSAGE_UPDATED_ID,
            json!({ "roles": ["assistant"] }),
        ),
        ("session::status-changed", ON_STATUS_CHANGED_ID, json!({})),
        ("harness::turn-completed", ON_TURN_COMPLETED_ID, json!({})),
    ];
    for (trigger_type, function_id, config) in bindings {
        bind_best_effort(iii, trigger_type, function_id, config);
    }
}

/// Bind approval trigger handlers. Only safe to call when the optional
/// `approval-gate` worker is connected (registers `approval::pending-*`).
pub fn bind_approval_triggers(iii: &Arc<IIIClient>) {
    let bindings = [
        (
            "approval::pending-created",
            ON_PENDING_CREATED_ID,
            json!({}),
        ),
        (
            "approval::pending-resolved",
            ON_PENDING_RESOLVED_ID,
            json!({}),
        ),
    ];
    for (trigger_type, function_id, config) in bindings {
        bind_best_effort(iii, trigger_type, function_id, config);
    }
}

/// Register the always-on control-plane HTTP triggers at boot.
///
/// Only the `set-webhook` control endpoint is static. The `webhook` ingress
/// route is registered/unregistered dynamically by [`crate::ingress`] to follow
/// the `updates` adapter (created on switch to webhook, removed on switch back
/// to polling) — see [`register_webhook_trigger`].
pub fn bind_http_triggers(iii: &Arc<IIIClient>) {
    let http = [(SET_WEBHOOK_ID, "telegram-bot/set-webhook", "POST")];
    for (function_id, api_path, http_method) in http {
        match iii.register_trigger(RegisterTriggerInput {
            trigger_type: "http".to_string(),
            function_id: function_id.to_string(),
            config: json!({ "api_path": api_path, "http_method": http_method }),
            metadata: None,
        }) {
            Ok(_) => tracing::info!(function_id, api_path, "http trigger registered"),
            Err(e) => tracing::warn!(error = %e, function_id, "failed to register http trigger"),
        }
    }
}

/// Register the Telegram webhook ingress HTTP route and return its [`Trigger`]
/// handle so the caller can later [`Trigger::unregister`] it. The route path is
/// [`crate::config::WEBHOOK_API_PATH`] — the same constant used to derive the
/// public URL handed to Telegram, so they cannot drift.
pub fn register_webhook_trigger(iii: &IIIClient) -> Result<Trigger, Error> {
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: WEBHOOK_ID.to_string(),
        config: json!({ "api_path": crate::config::WEBHOOK_API_PATH, "http_method": "POST" }),
        metadata: None,
    })
}

fn bind_best_effort(
    iii: &Arc<IIIClient>,
    trigger_type: &str,
    function_id: &str,
    config: serde_json::Value,
) {
    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: trigger_type.to_string(),
        function_id: function_id.to_string(),
        config,
        metadata: None,
    }) {
        Ok(_) => tracing::info!(trigger_type, function_id, "trigger binding requested"),
        Err(e) => tracing::warn!(
            trigger_type,
            function_id,
            error = %e,
            "trigger binding failed (sibling absent?)"
        ),
    }
}
