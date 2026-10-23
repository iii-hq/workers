//! The engine's worker change feed, driving the harness's recovery passes.
//!
//! Drift used to be found on timers: a 30 s binding sweep, a 120 s
//! orphaned-turn scan, 5-minute function/skill catalog reloads. Each of them
//! was really waiting for the same few things — a worker (the state store,
//! the queue, a trigger provider, another harness instance) connecting, going
//! away, or announcing its registrations — and the engine publishes exactly
//! those on `engine::workers-available`. This module binds that trigger once
//! and turns each event into coalesced kicks:
//!
//! | `event`                                      | kicks                                   |
//! |----------------------------------------------|-----------------------------------------|
//! | `worker_connected`, `worker_disconnected`    | binding pass, orphaned-turn pass        |
//! | `worker_metadata_updated` (registrations in) | the same, plus a catalog refresh        |
//!
//! The engine fires `worker_metadata_updated` only after the announcing
//! worker's buffered function/trigger registrations were applied, so it is
//! the moment a restarted worker's functions (and their schemas) are
//! readable — including this harness's own reconnect, since the SDK replays
//! this binding before announcing.
//!
//! A kick is a [`Notify`] permit: a burst of events while a pass runs
//! collapses into one follow-up pass, and no pass ever runs on a clock. The
//! same events bump a process-wide generation ([`subscribe_changes`]) that an
//! in-flight dispatch re-samples the engine epoch on, to notice an engine
//! restart without probing every second.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::RegisterFunction;
use serde_json::json;
use tokio::sync::Notify;

use crate::deps::Deps;

/// Internal handler the engine's worker change trigger calls.
pub const ENGINE_CHANGE_FN_ID: &str = "harness::on-engine-change";
/// Fired by the engine when a worker connects, disconnects, or announces its
/// metadata (`engine::workers::register`).
pub const WORKERS_AVAILABLE_TRIGGER: &str = "engine::workers-available";

/// Wake-ups for the passes the change feed drives. Each is consumed by one
/// long-lived driver task; see [`crate::bindings::expiry::run`],
/// [`crate::inflight::run`], and [`run_catalog`].
#[derive(Clone, Default)]
pub struct RecoveryKicks {
    /// Binding reconciliation: orphan delivery triggers, Compose wake
    /// recovery, and re-arming every stored binding's deadline timer.
    pub bindings: Arc<Notify>,
    /// Orphaned-turn re-drive.
    pub turns: Arc<Notify>,
    /// Function-registry (with schemas) and skill catalog refresh.
    pub catalog: Arc<Notify>,
}

/// Process-wide engine change generation, bumped on every
/// `engine::workers-available` event. Waiters that hold no [`Deps`] — an
/// in-flight dispatch watching for an engine restart — re-check once per
/// change instead of on a timer.
static ENGINE_CHANGES: std::sync::LazyLock<tokio::sync::watch::Sender<u64>> =
    std::sync::LazyLock::new(|| tokio::sync::watch::Sender::new(0));

/// Observe engine changes from now on. Subscribe BEFORE reading the state
/// being waited on, so a change between the read and the wait is not lost.
pub fn subscribe_changes() -> tokio::sync::watch::Receiver<u64> {
    ENGINE_CHANGES.subscribe()
}

fn note_engine_change() {
    ENGINE_CHANGES.send_modify(|generation| *generation = generation.wrapping_add(1));
}

/// What one `engine::workers-available` event reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineChange {
    WorkerConnected,
    WorkerDisconnected,
    /// `worker_metadata_updated`: the worker's registrations are applied.
    WorkerAnnounced,
    /// A tag this build does not know; treated like a connect/disconnect so a
    /// newer engine never silently stops driving recovery.
    Other,
}

impl EngineChange {
    pub fn parse(event: Option<&str>) -> Self {
        match event {
            Some("worker_connected") => Self::WorkerConnected,
            Some("worker_disconnected") => Self::WorkerDisconnected,
            Some("worker_metadata_updated") => Self::WorkerAnnounced,
            _ => Self::Other,
        }
    }
}

/// Every worker change can strand something the passes recover: a binding
/// whose record or provider moved, a turn whose step lived on the departed
/// worker (or never reached a queue that just came back). Only an announce
/// carries new registrations, so only it refreshes the catalogs.
pub fn kick(kicks: &RecoveryKicks, change: EngineChange) {
    kicks.bindings.notify_one();
    kicks.turns.notify_one();
    if change == EngineChange::WorkerAnnounced {
        kicks.catalog.notify_one();
    }
}

/// `harness::on-engine-change` payload. Only the event tag is read; the
/// `worker_id` is logged for tracing the cause of a pass.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct EngineChangeEvent {
    #[serde(default)]
    pub event: Option<String>,
    #[serde(default)]
    pub worker_id: Option<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct EngineChangeAck {
    pub ok: bool,
}

/// Register [`ENGINE_CHANGE_FN_ID`], bind it to `engine::workers-available`,
/// and start the catalog refresher. A failed bind only warns: every pass
/// still runs once at boot, it just will not re-run until restart.
pub fn register(deps: &Arc<Deps>) {
    let kicks = deps.kicks.clone();
    deps.iii.register_function(
        ENGINE_CHANGE_FN_ID,
        RegisterFunction::new_async(move |event: EngineChangeEvent| {
            let kicks = kicks.clone();
            async move {
                let change = EngineChange::parse(event.event.as_deref());
                tracing::debug!(
                    event = ?event.event,
                    worker_id = ?event.worker_id,
                    "engine worker change; kicking recovery passes"
                );
                kick(&kicks, change);
                note_engine_change();
                Ok::<_, Error>(EngineChangeAck { ok: true })
            }
        })
        .description(
            "Internal: re-run the harness's recovery passes (binding reconciliation, \
             orphaned-turn re-drive, catalog refresh) when engine workers connect, \
             disconnect, or announce their registrations.",
        )
        .metadata(json!({ "internal": true })),
    );
    match deps.iii.register_trigger(RegisterTriggerInput::new(
        WORKERS_AVAILABLE_TRIGGER.to_string(),
        ENGINE_CHANGE_FN_ID.to_string(),
        json!({}),
    )) {
        Ok(_) => tracing::info!(
            trigger_type = WORKERS_AVAILABLE_TRIGGER,
            function_id = ENGINE_CHANGE_FN_ID,
            "engine worker change trigger bound"
        ),
        Err(error) => tracing::warn!(
            trigger_type = WORKERS_AVAILABLE_TRIGGER,
            %error,
            "binding engine::workers-available failed; recovery passes run only at boot"
        ),
    }
    tokio::spawn(run_catalog(deps.clone()));
}

/// Refresh the function registry (re-reading every schema, which a worker
/// restart may have changed without changing the id set) and the skill
/// catalog once per announce.
pub async fn run_catalog(deps: Arc<Deps>) {
    loop {
        deps.kicks.catalog.notified().await;
        let timeout_ms = deps.cfg().await.dispatch_timeout_ms;
        let count = crate::discovery::refresh(&deps.iii, &deps.functions, timeout_ms).await;
        tracing::debug!(
            count,
            "function-registry cache refreshed on worker announce"
        );
        crate::skills::refresh(&deps.iii, &deps.skills, timeout_ms).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_tags_parse_and_unknown_ones_still_drive_recovery() {
        assert_eq!(
            EngineChange::parse(Some("worker_connected")),
            EngineChange::WorkerConnected
        );
        assert_eq!(
            EngineChange::parse(Some("worker_disconnected")),
            EngineChange::WorkerDisconnected
        );
        assert_eq!(
            EngineChange::parse(Some("worker_metadata_updated")),
            EngineChange::WorkerAnnounced
        );
        assert_eq!(EngineChange::parse(Some("future")), EngineChange::Other);
        assert_eq!(EngineChange::parse(None), EngineChange::Other);
    }

    async fn kicked(notify: &Notify) -> bool {
        tokio::time::timeout(std::time::Duration::from_millis(1), notify.notified())
            .await
            .is_ok()
    }

    #[tokio::test]
    async fn every_change_kicks_both_passes_and_only_an_announce_the_catalog() {
        for change in [
            EngineChange::WorkerConnected,
            EngineChange::WorkerDisconnected,
            EngineChange::Other,
        ] {
            let kicks = RecoveryKicks::default();
            kick(&kicks, change);
            assert!(kicked(&kicks.bindings).await, "{change:?}");
            assert!(kicked(&kicks.turns).await, "{change:?}");
            assert!(!kicked(&kicks.catalog).await, "{change:?}");
        }
        let kicks = RecoveryKicks::default();
        kick(&kicks, EngineChange::WorkerAnnounced);
        assert!(kicked(&kicks.bindings).await);
        assert!(kicked(&kicks.turns).await);
        assert!(kicked(&kicks.catalog).await);
    }

    #[tokio::test]
    async fn a_burst_of_changes_coalesces_into_one_pending_pass() {
        let kicks = RecoveryKicks::default();
        for _ in 0..5 {
            kick(&kicks, EngineChange::WorkerConnected);
        }
        assert!(
            kicked(&kicks.turns).await,
            "the burst leaves one pass pending"
        );
        assert!(!kicked(&kicks.turns).await, "...and only one");
    }
}
