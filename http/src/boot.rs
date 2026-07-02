//! Boot sequence: register the `http` trigger type (so the engine routes
//! trigger registrations through our [`HttpTriggerHandler`] into the shared
//! [`RouteTable`]) and start the HTTP server.

use std::net::SocketAddr;
use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterTriggerType};
use tokio::sync::{RwLock, oneshot};
use tokio::task::JoinHandle;

use crate::config::RestApiConfig;
use crate::configuration::{self, ConfigCell};
use crate::server::{self, RouterCell, ServerHandle};
use crate::trigger::{HttpTriggerHandler, RouteTable};
use crate::trigger_type;
use crate::types::{HttpRequest, HttpTriggerConfig};

/// Function id for `engine::workers::list`, used by [`guard_against_builtin_http`]
/// to detect whether the built-in `iii-http` worker is connected.
const LIST_WORKERS_FUNCTION_ID: &str = "engine::workers::list";

/// The worker id/name the built-in HTTP worker registers as.
const BUILTIN_III_HTTP_WORKER_ID: &str = "iii-http";

/// Handle to a running worker: the bound address, the shared route table, the
/// live config cell (so the caller can wire the `configuration:updated` trigger
/// and observe hot-reloads), the swappable router cell (so a same-address
/// config change can rebuild the CORS/timeout/concurrency layers live), and a
/// graceful-shutdown trigger.
pub struct BootHandle {
    pub local_addr: SocketAddr,
    pub routes: Arc<RwLock<RouteTable>>,
    pub config: ConfigCell,
    pub router: RouterCell,
    shutdown: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
}

impl BootHandle {
    /// Signal the server to stop and wait for the task to finish.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let _ = self.join.await;
    }
}

/// Register this worker's configured trigger type (see [`crate::trigger_type`])
/// and start the server. Returns once the listener is bound (its address
/// available in [`BootHandle::local_addr`]).
///
/// When the configured trigger type is `http`, refuses to start if the
/// built-in `iii-http` worker already owns it on the connected engine (see
/// [`guard_against_builtin_http`]) -- two owners of the same trigger type
/// collide (last-write-wins), so this turns the silent collision into a
/// fail-fast error.
pub async fn start(iii: Arc<IIIClient>, config: RestApiConfig) -> anyhow::Result<BootHandle> {
    let trigger_type = trigger_type();
    if trigger_type == "http" {
        guard_against_builtin_http(&iii).await?;
    }

    let cell = configuration::new_cell(config.normalized());

    let handler = HttpTriggerHandler::new();
    let routes = handler.routes.clone();

    // Registering the trigger type makes the engine deliver every trigger
    // binding of this type through `handler`, which populates `routes`.
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(trigger_type.as_str(), "HTTP API trigger", handler)
            .call_request_format::<HttpRequest>()
            .trigger_request_format::<HttpTriggerConfig>(),
    );

    let ServerHandle {
        local_addr,
        router,
        join,
        shutdown,
    } = server::serve(routes.clone(), iii, cell.clone()).await?;

    Ok(BootHandle {
        local_addr,
        routes,
        config: cell,
        router,
        shutdown: Some(shutdown),
        join,
    })
}

/// Query the engine for connected workers and bail out if the built-in
/// `iii-http` worker is active -- it already owns the `http` trigger type, so
/// registering it here would silently collide (last-write-wins).
async fn guard_against_builtin_http(iii: &Arc<IIIClient>) -> anyhow::Result<()> {
    let workers_list = iii
        .trigger(TriggerRequest {
            function_id: LIST_WORKERS_FUNCTION_ID.to_string(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: Some(5000),
        })
        .await
        .map_err(|e| anyhow::anyhow!("failed to query {LIST_WORKERS_FUNCTION_ID}: {e}"))?;

    if builtin_iii_http_active(&workers_list) {
        anyhow::bail!(
            "cannot register trigger type 'http': the built-in iii-http worker is active and \
             already owns it. Remove iii-http from the engine config, or run this worker with \
             III_HTTP_TRIGGER_TYPE=http-ng to coexist."
        );
    }

    Ok(())
}

/// Inspect the `{ "workers": [ { "id", "name", ... }, ... ] }` payload
/// returned by `engine::workers::list` and report whether the built-in
/// `iii-http` worker is among them (matched by `id` or `name`).
fn builtin_iii_http_active(workers_list: &serde_json::Value) -> bool {
    workers_list
        .get("workers")
        .and_then(|w| w.as_array())
        .is_some_and(|workers| {
            workers.iter().any(|worker| {
                worker.get("id").and_then(|v| v.as_str()) == Some(BUILTIN_III_HTTP_WORKER_ID)
                    || worker.get("name").and_then(|v| v.as_str())
                        == Some(BUILTIN_III_HTTP_WORKER_ID)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_builtin_iii_http_by_id() {
        let workers_list = json!({
            "workers": [
                { "id": "iii-http", "name": "http builtin", "status": "connected" },
                { "id": "some-other-worker", "name": "other", "status": "connected" },
            ]
        });
        assert!(builtin_iii_http_active(&workers_list));
    }

    #[test]
    fn detects_builtin_iii_http_by_name() {
        let workers_list = json!({
            "workers": [
                { "id": "worker-123", "name": "iii-http", "status": "connected" },
            ]
        });
        assert!(builtin_iii_http_active(&workers_list));
    }

    #[test]
    fn no_match_when_absent() {
        let workers_list = json!({
            "workers": [
                { "id": "shell", "name": "shell", "status": "connected" },
                { "id": "email", "name": "email", "status": "connected" },
            ]
        });
        assert!(!builtin_iii_http_active(&workers_list));
    }

    #[test]
    fn no_match_when_workers_empty() {
        let workers_list = json!({ "workers": [] });
        assert!(!builtin_iii_http_active(&workers_list));
    }

    #[test]
    fn no_match_when_workers_missing() {
        let workers_list = json!({});
        assert!(!builtin_iii_http_active(&workers_list));
    }

    #[test]
    fn trigger_type_defaults_to_http_ng() {
        // No test in this crate sets III_HTTP_TRIGGER_TYPE, so this only
        // asserts the default when the var is (as expected) unset -- avoids
        // mutating shared process env from a lib unit test.
        if std::env::var("III_HTTP_TRIGGER_TYPE").is_err() {
            assert_eq!(crate::trigger_type(), crate::DEFAULT_TRIGGER_TYPE);
        }
    }
}
