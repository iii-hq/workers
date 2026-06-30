//! Boot sequence: register the `http` trigger type (so the engine routes
//! trigger registrations through our [`HttpTriggerHandler`] into the shared
//! [`RouteTable`]) and start the HTTP server.

use std::net::SocketAddr;
use std::sync::Arc;

use iii_sdk::{IIIClient, RegisterTriggerType};
use tokio::sync::{RwLock, oneshot};
use tokio::task::JoinHandle;

use crate::TRIGGER_TYPE;
use crate::config::RestApiConfig;
use crate::server::{self, ServerHandle};
use crate::trigger::{HttpTriggerHandler, RouteTable};
use crate::types::{HttpRequest, HttpTriggerConfig};

/// Handle to a running worker: the bound address, the shared route table, and
/// a graceful-shutdown trigger.
pub struct BootHandle {
    pub local_addr: SocketAddr,
    pub routes: Arc<RwLock<RouteTable>>,
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

/// Register the `http` trigger type and start the server. Returns once the
/// listener is bound (its address available in [`BootHandle::local_addr`]).
pub async fn start(iii: Arc<IIIClient>, config: RestApiConfig) -> anyhow::Result<BootHandle> {
    let config = config.normalized();

    let handler = HttpTriggerHandler::new();
    let routes = handler.routes.clone();

    // Registering the trigger type makes the engine deliver every `http`
    // trigger binding through `handler`, which populates `routes`.
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(TRIGGER_TYPE, "HTTP API trigger", handler)
            .call_request_format::<HttpRequest>()
            .trigger_request_format::<HttpTriggerConfig>(),
    );

    let ServerHandle {
        local_addr,
        join,
        shutdown,
    } = server::serve(routes.clone(), iii, config).await?;

    Ok(BootHandle {
        local_addr,
        routes,
        shutdown: Some(shutdown),
        join,
    })
}
