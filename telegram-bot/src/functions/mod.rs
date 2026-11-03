pub mod bindings;
pub mod notify;
pub mod set_webhook;
pub mod webhook;

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{Deliver, IIIClient, ObserverHandle, RegisterFunction, Source};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

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

/// The observed functions of the http worker, session-manager, the harness
/// and approval-gate.
pub const HTTP_ON_REQUEST_FN: &str = "http::on-request";
pub const SESSION_MESSAGE_ADDED_FN: &str = "session::on-message-added";
pub const SESSION_MESSAGE_UPDATED_FN: &str = "session::on-message-updated";
pub const SESSION_STATUS_CHANGED_FN: &str = "session::on-status-changed";
pub const HARNESS_TURN_COMPLETED_FN: &str = "harness::on-turn-completed";
pub const APPROVAL_PENDING_CREATED_FN: &str = "approval::on-pending-created";
pub const APPROVAL_PENDING_RESOLVED_FN: &str = "approval::on-pending-resolved";

/// API path of the always-on `set-webhook` control route.
pub const SET_WEBHOOK_API_PATH: &str = "telegram-bot/set-webhook";

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

/// Messages whose role is one of `roles`.
fn roles_filter(roles: &[&str]) -> Value {
    json!({ "input.message.role": { "$in": roles } })
}

/// `(source, target, filters)` of the session and harness observers.
pub fn session_observers() -> [(&'static str, &'static str, Option<Value>); 4] {
    [
        (
            SESSION_MESSAGE_ADDED_FN,
            ON_MESSAGE_ADDED_ID,
            Some(roles_filter(&["assistant", "function_result"])),
        ),
        (
            SESSION_MESSAGE_UPDATED_FN,
            ON_MESSAGE_UPDATED_ID,
            Some(roles_filter(&["assistant"])),
        ),
        (SESSION_STATUS_CHANGED_FN, ON_STATUS_CHANGED_ID, None),
        (HARNESS_TURN_COMPLETED_FN, ON_TURN_COMPLETED_ID, None),
    ]
}

/// `(source, target, filters)` of the approval observers.
pub fn approval_observers() -> [(&'static str, &'static str, Option<Value>); 2] {
    [
        (APPROVAL_PENDING_CREATED_FN, ON_PENDING_CREATED_ID, None),
        (APPROVAL_PENDING_RESOLVED_FN, ON_PENDING_RESOLVED_ID, None),
    ]
}

/// Observe the session and harness events, and the approval events of the
/// optional `approval-gate` worker. Each observer stays `pending` while its
/// worker is absent and becomes active when that worker registers the
/// function, so a gate that connects later is observed with no extra step.
/// Targets receive the event itself (`deliver: output`).
pub fn observe_events(iii: &Arc<IIIClient>) {
    for (source, target, filters) in session_observers().into_iter().chain(approval_observers()) {
        observe_event(iii, source, target, filters);
    }
}

/// The observer lives as long as the worker's connection; dropping the
/// handle does not dispose it.
fn observe_event(iii: &IIIClient, source: &str, target: &str, filters: Option<Value>) {
    let mut src = Source::function(source).deliver(Deliver::Output);
    if let Some(filters) = filters {
        src = src.filters(filters);
    }
    let handle = iii.observe(src, target);
    tracing::info!(
        function_id = source,
        observer_target = target,
        observer_id = handle.id(),
        "observing"
    );
}

/// The args of the `http::on-request` observer that serves `POST /<api_path>`.
pub fn route_args(api_path: &str) -> Value {
    json!({ "method": "POST", "path": format!("/{api_path}") })
}

/// Observe the always-on control-plane HTTP route at boot.
///
/// Only the `set-webhook` control endpoint is static. The `webhook` ingress
/// route is observed and disposed by [`crate::ingress`] to follow the
/// `updates` adapter (observed on switch to webhook, disposed on switch back
/// to polling) — see [`observe_webhook_route`].
pub fn observe_control_routes(iii: &Arc<IIIClient>) {
    let handle = iii.observe(
        Source::function(HTTP_ON_REQUEST_FN).args(route_args(SET_WEBHOOK_API_PATH)),
        SET_WEBHOOK_ID,
    );
    tracing::info!(
        function_id = SET_WEBHOOK_ID,
        api_path = SET_WEBHOOK_API_PATH,
        observer_id = handle.id(),
        "http route observed"
    );
}

/// Observe `http::on-request` with the Telegram webhook ingress route and
/// return the handle so the caller can later dispose it. The route path is
/// [`crate::config::WEBHOOK_API_PATH`] — the same constant used to derive the
/// public URL handed to Telegram, so they cannot drift.
pub fn observe_webhook_route(iii: &IIIClient) -> ObserverHandle {
    iii.observe(
        Source::function(HTTP_ON_REQUEST_FN).args(route_args(crate::config::WEBHOOK_API_PATH)),
        WEBHOOK_ID,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_are_posts_under_the_worker_prefix() {
        assert_eq!(
            route_args(crate::config::WEBHOOK_API_PATH),
            json!({ "method": "POST", "path": "/telegram-bot/webhook" })
        );
        assert_eq!(
            route_args(SET_WEBHOOK_API_PATH),
            json!({ "method": "POST", "path": "/telegram-bot/set-webhook" })
        );
    }

    #[test]
    fn message_observers_filter_on_the_message_role() {
        let [added, updated, status, turn] = session_observers();
        assert_eq!(
            added.2,
            Some(json!({ "input.message.role": { "$in": ["assistant", "function_result"] } }))
        );
        assert_eq!(
            updated.2,
            Some(json!({ "input.message.role": { "$in": ["assistant"] } }))
        );
        assert_eq!(status.2, None);
        assert_eq!(turn.2, None);
    }

    #[test]
    fn observed_functions_are_the_v2_event_sources() {
        let sources: Vec<_> = session_observers()
            .into_iter()
            .chain(approval_observers())
            .map(|(source, target, _)| (source, target))
            .collect();
        assert_eq!(
            sources,
            vec![
                (
                    "session::on-message-added",
                    "telegram-bot::on-message-added"
                ),
                (
                    "session::on-message-updated",
                    "telegram-bot::on-message-updated"
                ),
                (
                    "session::on-status-changed",
                    "telegram-bot::on-status-changed"
                ),
                (
                    "harness::on-turn-completed",
                    "telegram-bot::on-turn-completed"
                ),
                (
                    "approval::on-pending-created",
                    "telegram-bot::on-pending-created"
                ),
                (
                    "approval::on-pending-resolved",
                    "telegram-bot::on-pending-resolved"
                ),
            ]
        );
    }
}
