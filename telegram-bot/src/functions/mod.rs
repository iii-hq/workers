pub mod bindings;
pub mod set_webhook;
pub mod webhook;

use std::future::Future;
use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, RegisterTriggerInput, III};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::json;

use crate::deps::Deps;

pub const WEBHOOK_ID: &str = "telegram-bot::webhook";
pub const SET_WEBHOOK_ID: &str = "telegram-bot::set-webhook";
pub const ON_MESSAGE_ADDED_ID: &str = "telegram-bot::on-message-added";
pub const ON_MESSAGE_UPDATED_ID: &str = "telegram-bot::on-message-updated";
pub const ON_STATUS_CHANGED_ID: &str = "telegram-bot::on-status-changed";
pub const ON_TURN_COMPLETED_ID: &str = "telegram-bot::on-turn-completed";
pub const ON_PENDING_CREATED_ID: &str = "telegram-bot::on-pending-created";
pub const ON_PENDING_RESOLVED_ID: &str = "telegram-bot::on-pending-resolved";

fn register<Req, Resp, F, Fut>(
    iii: &Arc<III>,
    deps: &Arc<Deps>,
    id: &str,
    description: &str,
    handler: F,
) where
    Req: DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Deps>, Req) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<Resp, IIIError>> + Send + 'static,
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

pub fn register_all(iii: &Arc<III>, deps: &Arc<Deps>) {
    webhook::register(iii, deps);
    set_webhook::register(iii, deps);
    bindings::register(iii, deps);
    tracing::info!("all functions registered");
}

pub fn bind_triggers(iii: &Arc<III>) {
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

pub fn bind_http_triggers(iii: &Arc<III>) {
    let http = [
        (WEBHOOK_ID, "telegram-bot/webhook", "POST"),
        (SET_WEBHOOK_ID, "telegram-bot/set-webhook", "POST"),
    ];
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

fn bind_best_effort(
    iii: &Arc<III>,
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
