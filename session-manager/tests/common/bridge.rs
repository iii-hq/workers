//! In-process "bridged instance" stacks for @engine bridge scenarios.
//!
//! Each stack is exactly what a bridge-adapter binary wires up:
//! a `BridgeStore` + `RemotePublisher` over the (shared test) engine
//! connection, a local `Emitter` fed by a relay observing the main's
//! `session::store::on-events` feed — except the "local bus" deliverer
//! is a recorder, so scenarios add local observers to it and assert what
//! they received.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::IIIClient;

use session_manager::config::WorkerConfig;
use session_manager::events::{
    attach_bridge_relay, BridgeRelay, Emitter, EventSink, RemotePublisher,
};
use session_manager::functions::Deps;
use session_manager::service::SessionService;
use session_manager::store::{BridgeStore, SessionStore};

use super::recorder::{Recorder, RecorderDeliverer};

const BRIDGE_TIMEOUT_MS: u64 = 5_000;

pub struct BridgeStack {
    pub deps: Arc<Deps>,
    /// The instance's "local bus": add local observers here, and read what
    /// they received.
    pub recorder: Recorder,
    /// The relay function and its observer of the main's feed; disposed
    /// after the scenario.
    pub relay: BridgeRelay,
}

/// Build one bridged-instance stack against `engine` (the main's bus).
pub async fn build_bridge_stack(engine: &Arc<IIIClient>) -> BridgeStack {
    let recorder = Recorder::new();
    let emitter = Arc::new(Emitter::new(Arc::new(RecorderDeliverer(recorder.clone()))));
    let relay = attach_bridge_relay(engine, emitter);

    let store: Arc<dyn SessionStore> =
        Arc::new(BridgeStore::new(engine.clone(), BRIDGE_TIMEOUT_MS));
    let service = Arc::new(SessionService::new(store, &WorkerConfig::default()));
    let sink: Arc<dyn EventSink> =
        Arc::new(RemotePublisher::new(engine.clone(), BRIDGE_TIMEOUT_MS));

    // The relay receives events once its observer of the main's feed is
    // active. The main stack runs in-process (common::workers), so the
    // observer activates as soon as the engine has both functions.
    match tokio::time::timeout(Duration::from_secs(10), relay.observer.ready()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => panic!(
            "bridge relay {} could not observe the main's event feed: {e}",
            relay.function_id
        ),
        Err(_) => panic!(
            "bridge relay {} did not start observing the main's event feed within 10s",
            relay.function_id
        ),
    }

    BridgeStack {
        deps: Arc::new(Deps { service, sink }),
        recorder,
        relay,
    }
}
