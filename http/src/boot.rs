//! Boot sequence: register the `http::on-request` function (whose observers'
//! routes land in the shared [`RouteTable`], see [`crate::on_request`]) and
//! start the HTTP server.

use std::net::SocketAddr;
use std::sync::Arc;

use iii_sdk::runtime::FunctionRef;
use iii_sdk::IIIClient;
use tokio::sync::RwLock;

use crate::config::RestApiConfig;
use crate::configuration::{self, ApplyLock, ConfigCell};
use crate::inflight::InFlightRequests;
use crate::on_request;
use crate::routes::RouteTable;
use crate::server::{self, HotRouter, RouterCell, ServerControlCell, ServerHandle};

/// Handle to a running worker: the shared route table, the live config cell (so
/// the caller can observe `configuration::on-updated` and follow
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
/// [`configuration::register_config_observer`], which serializes overlapping
/// `http::on-config-change` runs onto it (see [`ApplyLock`]).
pub struct BootHandle {
    pub iii: Arc<IIIClient>,
    pub local_addr: SocketAddr,
    pub routes: Arc<RwLock<RouteTable>>,
    /// `http::on-request`; its observers' routes are in `routes`.
    pub on_request: FunctionRef,
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

    /// Current restricted listener address; None when disabled or shut down.
    pub async fn current_webhook_addr(&self) -> Option<SocketAddr> {
        let webhook = self.hot_router.webhook.as_ref()?;
        webhook.control.lock().await.as_ref().map(|c| c.local_addr)
    }

    /// Serialize shutdown against reload, drain both listeners, and join them,
    /// then unregister `http::on-request` and clear its hooks (the engine
    /// holds its observers until it is registered again, by a restarted
    /// worker or a later [`start`] on the same client).
    pub async fn shutdown(self) {
        let _guard = self.apply_lock.lock().await;
        let normal = self.control.lock().await.take();
        let webhook = match &self.hot_router.webhook {
            Some(webhook) => webhook.control.lock().await.take(),
            None => None,
        };
        let stop = |control| async move {
            if let Some(control) = control {
                server::stop_server(control).await;
            }
        };
        tokio::join!(stop(normal), stop(webhook));
        self.on_request.unregister();
        on_request::clear_hooks(&self.iii);
    }
}

/// Register this worker's function ([`on_request::ON_REQUEST_FN`]) and start
/// the server. Returns once the listener is bound (its address available in
/// [`BootHandle::local_addr`]).
pub async fn start(iii: Arc<IIIClient>, config: RestApiConfig) -> anyhow::Result<BootHandle> {
    let cell = configuration::new_cell(config.normalized());
    let apply_lock: ApplyLock = Arc::new(tokio::sync::Mutex::new(
        configuration::ReloadState::default(),
    ));

    // Observers of `http::on-request` add their routes to this table, which
    // its deliver hook matches requests against. The server and the hook
    // share the requests in flight.
    let routes: Arc<RwLock<RouteTable>> = Arc::default();
    let requests = InFlightRequests::default();
    let on_request = on_request::register(&iii, routes.clone(), requests.clone(), cell.clone());

    let ServerHandle {
        local_addr,
        router,
        hot_router,
        control,
    } = match server::serve(routes.clone(), requests, iii.clone(), cell.clone()).await {
        Ok(handle) => handle,
        Err(error) => {
            on_request.unregister();
            on_request::clear_hooks(&iii);
            return Err(error);
        }
    };

    Ok(BootHandle {
        iii,
        local_addr,
        routes,
        on_request,
        config: cell,
        router,
        hot_router,
        control,
        apply_lock,
    })
}
