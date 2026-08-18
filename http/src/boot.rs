//! Boot sequence: register the `http` trigger type (so the engine routes
//! trigger registrations through our [`HttpTriggerHandler`] into the shared
//! [`RouteTable`]) and start the HTTP server.

use std::net::SocketAddr;
use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterTriggerType};
use tokio::sync::RwLock;

use crate::config::RestApiConfig;
use crate::configuration::{self, ApplyLock, ConfigCell};
use crate::server::{self, HotRouter, RouterCell, ServerControlCell, ServerHandle};
use crate::trigger::{HttpTriggerHandler, RouteTable};
use crate::types::{HttpRequest, HttpTriggerConfig};
use crate::TRIGGER_TYPE;

/// Function id for `engine::workers::list`, used by [`guard_against_builtin_http`]
/// to detect whether the built-in `iii-http` worker is connected.
const LIST_WORKERS_FUNCTION_ID: &str = "engine::workers::list";

/// The worker id/name the built-in HTTP worker registers as.
const BUILTIN_III_HTTP_WORKER_ID: &str = "iii-http";

/// Handle to a running worker: the shared route table, the live config cell (so
/// the caller can wire the `configuration:updated` trigger and observe
/// hot-reloads), the swappable router cell (so a same-address config change can
/// rebuild the CORS/timeout/concurrency layers live), the shared [`HotRouter`]
/// and [`ServerControlCell`] (so a host/port change can rebind the listener),
/// plus the address the server bound at boot.
///
/// `local_addr` is the INITIAL bound address; after a host/port rebind
/// (Phase B) the live address changes — read the current one via
/// [`BootHandle::current_addr`] or from the config cell.
///
/// `apply_lock` is shared with the caller so it can be threaded into
/// [`configuration::register_config_trigger`], which serializes overlapping
/// `http::on-config-change` runs onto it (see [`ApplyLock`]).
pub struct BootHandle {
    pub local_addr: SocketAddr,
    pub routes: Arc<RwLock<RouteTable>>,
    pub config: ConfigCell,
    pub router: RouterCell,
    pub hot_router: HotRouter,
    pub control: ServerControlCell,
    pub apply_lock: ApplyLock,
}

impl BootHandle {
    /// The address the currently-running server is bound to. Differs from
    /// [`BootHandle::local_addr`] after a host/port rebind. `None` once the
    /// server has been shut down.
    pub async fn current_addr(&self) -> Option<SocketAddr> {
        self.control.lock().await.as_ref().map(|c| c.local_addr)
    }

    /// Gracefully stop the running server and wait for its task to finish.
    pub async fn shutdown(self) {
        if let Some(control) = self.control.lock().await.take() {
            let _ = control.graceful.send(());
            let _ = control.join.await;
        }
    }
}

/// Register this worker's trigger type ([`crate::TRIGGER_TYPE`]) and start
/// the server. Returns once the listener is bound (its address available in
/// [`BootHandle::local_addr`]).
///
/// Always refuses to start if the built-in `iii-http` worker already owns
/// the `http` trigger type on the connected engine (see
/// [`guard_against_builtin_http`]) -- two owners of the same trigger type
/// collide (last-write-wins), so this turns the silent collision into a
/// fail-fast error.
pub async fn start(iii: Arc<IIIClient>, config: RestApiConfig) -> anyhow::Result<BootHandle> {
    guard_against_builtin_http(&iii).await?;

    let cell = configuration::new_cell(config.normalized());
    let apply_lock: ApplyLock = Arc::new(tokio::sync::Mutex::new(()));

    let handler = HttpTriggerHandler::new();
    let routes = handler.routes.clone();

    // Registering the trigger type makes the engine deliver every trigger
    // binding of this type through `handler`, which populates `routes`.
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(TRIGGER_TYPE, "HTTP API trigger", handler)
            .call_request_format::<HttpRequest>()
            .trigger_request_format::<HttpTriggerConfig>(),
    );

    let ServerHandle {
        local_addr,
        router,
        hot_router,
        control,
    } = server::serve(routes.clone(), iii, cell.clone()).await?;

    Ok(BootHandle {
        local_addr,
        routes,
        config: cell,
        router,
        hot_router,
        control,
        apply_lock,
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
            "cannot start the http worker: the built-in iii-http worker is active and owns the \
             'http' trigger type. Remove iii-http from the engine config (a config.yaml that \
             doesn't list it won't run it), then start this worker."
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
}
