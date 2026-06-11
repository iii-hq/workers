//! In-process "bridged instance" stacks for @engine bridge scenarios.
//!
//! Each stack is exactly what a `backend: bridge` binary wires up:
//! a `BridgeStore` + `RemotePublisher` over the (shared test) engine
//! connection, a local `Emitter` fed by a relay attached to the main's
//! `session::store::events` feed — except the "local bus" deliverer is
//! a recorder so scenarios can assert what local subscribers received.

use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::III;

use session_manager::config::WorkerConfig;
use session_manager::events::{
    attach_bridge_relay, Emitter, EventSink, RemotePublisher, TriggerSets,
};
use session_manager::functions::Deps;
use session_manager::service::SessionService;
use session_manager::store::{BridgeStore, SessionStore};

use super::recorder::{Recorder, RecorderDeliverer};

const BRIDGE_TIMEOUT_MS: u64 = 5_000;

pub struct BridgeStack {
    pub deps: Arc<Deps>,
    /// The instance's local emitter (bind local subscribers to its sets).
    pub emitter: Arc<Emitter>,
    /// What the instance's "local bus" delivered.
    pub recorder: Recorder,
    /// Kept for diagnostics when a propagation assertion fails.
    #[allow(dead_code)]
    pub relay_fn: String,
}

/// Build one bridged-instance stack against `engine` (the main's bus).
pub async fn build_bridge_stack(engine: &Arc<III>) -> BridgeStack {
    let recorder = Recorder::new();
    let emitter = Arc::new(Emitter::new(
        TriggerSets::new(),
        Arc::new(RecorderDeliverer(recorder.clone())),
    ));
    let relay_fn = attach_bridge_relay(engine, emitter.clone());

    let store: Arc<dyn SessionStore> =
        Arc::new(BridgeStore::new(engine.clone(), BRIDGE_TIMEOUT_MS));
    let service = Arc::new(SessionService::new(store, &WorkerConfig::default()));
    let sink: Arc<dyn EventSink> =
        Arc::new(RemotePublisher::new(engine.clone(), BRIDGE_TIMEOUT_MS));

    // The relay only delivers once its STORE_EVENTS binding has
    // round-tripped through the engine to the main's handler. The main
    // stack runs in-process (common::workers), so poll its live relay
    // registry until this stack's relay id appears — no fixed sleep.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let attached = super::workers::shared()
            .is_some_and(|shared| shared.bridges.function_ids().contains(&relay_fn));
        if attached {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "bridge relay {relay_fn} did not attach to the main's event feed within 10s"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    BridgeStack {
        deps: Arc::new(Deps { service, sink }),
        emitter,
        recorder,
        relay_fn,
    }
}
