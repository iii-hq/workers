//! request_id → in-flight upstream cancel, backing `provider::<id>::abort`.
//! Provider-side mirror of the router's `chat::inflight`: the stream fn
//! registers its request id before spawning the upstream, the abort function
//! signals it, and `pump_abortable` returns on the signal — dropping the
//! upstream receiver, which aborts the upstream task and its in-flight HTTP
//! request. Level-triggered watch channels: an abort that lands between
//! `watch` and the pump's first poll is still observed. `abort` signals
//! existing entries only (no create), so stray aborts for finished requests
//! can't leak entries — the ping-write backstop in the pump covers that race.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use tokio::sync::watch;

use crate::types::router::{ProviderAbortRequest, ProviderAbortResponse};

#[derive(Clone, Default)]
pub struct StreamAborts {
    map: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
}

impl StreamAborts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe for `request_id`, creating the entry. Call before spawning
    /// the upstream; pair with `remove` when the stream ends.
    pub fn watch(&self, request_id: &str) -> watch::Receiver<bool> {
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        map.entry(request_id.to_string())
            .or_insert_with(|| watch::channel(false).0)
            .subscribe()
    }

    /// Signal cancellation. `false` when the request is unknown — already
    /// finished, never started, or previously aborted (idempotent).
    pub fn abort(&self, request_id: &str) -> bool {
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        match map.remove(request_id) {
            Some(tx) => {
                tx.send_replace(true);
                true
            }
            None => false,
        }
    }

    /// Drop `request_id`'s entry once its stream is over.
    pub fn remove(&self, request_id: &str) {
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        map.remove(request_id);
    }

    /// RAII registration: subscribe on create, deregister on Drop. Register
    /// BEFORE the stream fn's first await so an abort during setup latches,
    /// and hold the guard across the call so cleanup runs on every exit —
    /// including the executor cancelling the handler future mid-await, which
    /// sequential cleanup after the call would silently miss.
    pub fn register(&self, request_id: &str) -> AbortGuard {
        AbortGuard {
            rx: self.watch(request_id),
            aborts: self.clone(),
            request_id: request_id.to_string(),
        }
    }
}

pub struct AbortGuard {
    aborts: StreamAborts,
    request_id: String,
    rx: watch::Receiver<bool>,
}

impl AbortGuard {
    /// A receiver for `pump_abortable` (level-triggered; clone per pump).
    pub fn watch(&self) -> watch::Receiver<bool> {
        self.rx.clone()
    }

    /// True once the abort fired (also observable via a pre-fired `watch`).
    pub fn is_fired(&self) -> bool {
        *self.rx.borrow()
    }
}

impl Drop for AbortGuard {
    fn drop(&mut self) {
        // A fired abort already consumed the entry — harmless no-op then.
        self.aborts.remove(&self.request_id);
    }
}

/// The `provider::<id>::abort` handler, ready to register: signals the
/// stream registered under `request_id` (see `StreamAborts`).
pub fn make_abort(
    aborts: StreamAborts,
) -> impl Fn(ProviderAbortRequest) -> BoxFuture<'static, Result<ProviderAbortResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |req: ProviderAbortRequest| {
        let aborted = aborts.abort(&req.request_id);
        Box::pin(async move { Ok(ProviderAbortResponse { aborted }) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abort_signals_registered_watchers_once() {
        let aborts = StreamAborts::new();
        let rx = aborts.watch("r1");
        assert!(!*rx.borrow());
        assert!(aborts.abort("r1"));
        assert!(*rx.borrow());
        // entry consumed: a second abort is an idempotent no-op
        assert!(!aborts.abort("r1"));
    }

    #[test]
    fn abort_of_unknown_request_is_a_noop() {
        let aborts = StreamAborts::new();
        assert!(!aborts.abort("never-registered"));
    }

    /// The sender lives in the registry (not the subscriber), so a watcher
    /// subscribing and dropping doesn't invalidate a later abort signal.
    #[test]
    fn resubscribing_sees_a_pre_subscribe_state() {
        let aborts = StreamAborts::new();
        drop(aborts.watch("r1"));
        let rx = aborts.watch("r1");
        assert!(aborts.abort("r1"));
        assert!(*rx.borrow());
    }

    /// The RAII property the providers rely on: dropping the guard (any exit,
    /// including a cancelled future) deregisters; a pre-drop abort still
    /// reaches the guard's receiver.
    #[test]
    fn guard_deregisters_on_drop_and_latches_aborts() {
        let aborts = StreamAborts::new();
        let guard = aborts.register("r1");
        assert!(!guard.is_fired());
        assert!(aborts.abort("r1"));
        assert!(guard.is_fired());
        assert!(*guard.watch().borrow());
        drop(guard);
        assert!(!aborts.abort("r1"), "entry gone after guard drop");

        let guard2 = aborts.register("r2");
        drop(guard2);
        assert!(!aborts.abort("r2"), "unfired entry removed by guard drop");
    }
}
