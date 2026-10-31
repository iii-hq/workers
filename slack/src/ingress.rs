//! Bridge ingress reconciliation. Two inbound transports, picked by config:
//!
//! - **Socket Mode** (`app_token` set): the worker dials out to Slack over a
//!   WebSocket. No public URL needed. Takes precedence.
//! - **HTTP / Events API** (`signing_secret` + `public_base_url`): observe
//!   `http::on-request` with the `POST /slack/events` and
//!   `POST /slack/interactions` routes. The http worker calls the target with
//!   each request, and the target streams the response.
//!
//! Driven at boot and on every config reload.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::{IIIClient, ObserverHandle, Source};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::clients::socket;
use crate::config::{EVENTS_API_PATH, INTERACTIONS_API_PATH};
use crate::deps::Deps;
use crate::functions::{events::EVENTS_ID, interactions::INTERACTIONS_ID};

/// The http worker's request function; each observer serves one route.
pub const ON_REQUEST_FN: &str = "http::on-request";

/// How long a new route observer may stay `pending` before it is reported.
const ROUTE_READY_WAIT: Duration = Duration::from_secs(10);

/// Reconcile both inbound transports to the current config, and refresh the
/// cached Slack identity (the bot user id the bridge needs for mention handling).
pub async fn apply(deps: &Arc<Deps>) {
    match crate::clients::slack::auth_test(deps).await {
        Ok(id) => *deps.identity.write().await = Some(id),
        Err(e) => {
            // Keep the last-known-good identity: a transient auth.test failure on
            // reload must not blind mention/self detection. An explicit token
            // problem surfaces via `slack::config-status` (which does clear it).
            tracing::warn!(error = %e, "failed to refresh Slack identity (keeping cached)");
        }
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
    let mut guard = deps.bridge_routes.lock().await;
    if enabled && guard.is_empty() {
        for (id, path) in [
            (EVENTS_ID, EVENTS_API_PATH),
            (INTERACTIONS_ID, INTERACTIONS_API_PATH),
        ] {
            let route = observe_route(&deps.iii, id, path);
            report_route(route.clone(), id, path);
            guard.push(route);
        }
        if let Some(url) = deps.cfg().await.events_url() {
            tracing::info!(events_url = %url, "http bridge enabled — point Slack Event Subscriptions here");
        }
    } else if !enabled && !guard.is_empty() {
        dispose_routes(guard.drain(..)).await;
        tracing::info!("http bridge disabled — routes removed");
    }
}

/// Stop both transports on shutdown.
pub async fn shutdown(deps: &Arc<Deps>) {
    if let Some(cancel) = deps.socket_cancel.lock().await.take() {
        cancel.cancel();
    }
    let mut guard = deps.bridge_routes.lock().await;
    dispose_routes(guard.drain(..)).await;
}

/// The args of the `http::on-request` observer that serves `POST /<api_path>`.
pub fn route_args(api_path: &str) -> Value {
    json!({ "method": "POST", "path": format!("/{api_path}") })
}

fn observe_route(iii: &IIIClient, function_id: &str, api_path: &str) -> ObserverHandle {
    iii.observe(
        Source::function(ON_REQUEST_FN).args(route_args(api_path)),
        function_id,
    )
}

/// Logs once whether the route became active. The observer stays `pending`
/// while the http worker is absent and serves the route once it registers;
/// it ends in `error` when the http worker refuses it (`HTTP_ROUTE_CONFLICT`).
fn report_route(route: ObserverHandle, function_id: &'static str, api_path: &'static str) {
    tokio::spawn(async move {
        match tokio::time::timeout(ROUTE_READY_WAIT, route.ready()).await {
            Ok(Ok(())) => {
                tracing::info!(function_id, api_path, "bridge route registered")
            }
            Ok(Err(e)) => {
                tracing::error!(error = %e, function_id, api_path, "bridge route refused")
            }
            Err(_) => tracing::warn!(
                function_id,
                api_path,
                "bridge route pending (is the http worker running?)"
            ),
        }
    });
}

async fn dispose_routes(routes: impl Iterator<Item = ObserverHandle>) {
    for route in routes {
        if let Err(e) = route.dispose().await {
            tracing::warn!(error = %e, observer_id = route.id(), "bridge route dispose failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_are_posts_under_the_slack_prefix() {
        assert_eq!(
            route_args(EVENTS_API_PATH),
            json!({ "method": "POST", "path": "/slack/events" })
        );
        assert_eq!(
            route_args(INTERACTIONS_API_PATH),
            json!({ "method": "POST", "path": "/slack/interactions" })
        );
    }
}
