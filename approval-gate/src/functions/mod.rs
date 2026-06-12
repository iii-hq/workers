//! The `approval::*` functions. Each `<verb>.rs` holds a
//! `pub async fn handle(deps, req)` that the registration closure wraps;
//! tests call the same `handle` functions directly, so engine-free tests
//! exercise the exact production code path.

pub mod approve_always;
pub mod clear_settings;
pub mod gate;
pub mod get_pending;
pub mod get_settings;
pub mod list_pending;
pub mod on_config_change;
pub mod on_session_deleted;
pub mod on_turn_completed;
pub mod purge;
pub mod resolve;
pub mod set_mode;
pub mod sweep;

pub mod add_always_allow;
pub mod remove_always_allow;

use std::future::Future;
use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, III};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::bus::Bus;
use crate::config::WorkerConfig;
use crate::error::ApprovalError;
use crate::events::EventSink;
use crate::gate_config::SharedDefaults;

/// Everything a function handler needs.
pub struct Deps {
    pub bus: Arc<dyn Bus>,
    pub sink: Arc<dyn EventSink>,
    pub defaults: SharedDefaults,
    pub cfg: Arc<WorkerConfig>,
}

/// Register one typed handler under `id`, mapping `ApprovalError` into
/// the bus error shape (`code: message`).
fn register<Req, Resp, F, Fut>(
    iii: &Arc<III>,
    deps: &Arc<Deps>,
    id: &str,
    description: &str,
    handler: F,
) where
    Req: DeserializeOwned + JsonSchema + Send + 'static,
    Resp: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Deps>, Req) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<Resp, ApprovalError>> + Send + 'static,
{
    let deps = deps.clone();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |req: Req| {
            let deps = deps.clone();
            let handler = handler.clone();
            async move { handler(deps, req).await.map_err(IIIError::from) }
        })
        .description(description),
    );
}

pub fn register_all(iii: &Arc<III>, deps: &Arc<Deps>) {
    register(
        iii,
        deps,
        "approval::gate",
        "pre_dispatch hook: evaluate the permission model and answer continue / deny / hold; writes the pending inbox record on hold. Called by the harness only.",
        |d, r| async move { gate::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "approval::resolve",
        "Apply a human decision to a held call: release it for execution (allow) or deliver a denial (deny). Human/console-only.",
        |d, r| async move { resolve::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "approval::list_pending",
        "The pending inbox across sessions, with tenancy filters; the catch-up path for notification workers after a restart.",
        |d, r| async move { list_pending::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "approval::get_pending",
        "Read one pending record; null when resolved or unknown.",
        |d, r| async move { get_pending::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "approval::set_mode",
        "Set the session's permission mode (manual / auto / full). Human/console-only.",
        |d, r| async move { set_mode::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "approval::add_always_allow",
        "Add a function to the session's auto-mode trust list (idempotent). Human/console-only.",
        |d, r| async move { add_always_allow::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "approval::remove_always_allow",
        "Remove a function from the session's auto-mode trust list (no-op when absent). Human/console-only.",
        |d, r| async move { remove_always_allow::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "approval::approve_always",
        "Record a per-session 'approve always' grant (honoured in every mode). Human/console-only.",
        |d, r| async move { approve_always::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "approval::get_settings",
        "Read the session's effective settings (stored record or configuration defaults); never writes.",
        |d, r| async move { get_settings::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "approval::clear_settings",
        "Drop the session's stored settings record (revert to configuration defaults).",
        |d, r| async move { clear_settings::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "approval::on_config_change",
        "Internal: configuration trigger handler (reload deployment defaults).",
        |d, r| async move { on_config_change::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "approval::on_session_deleted",
        "Internal: session::deleted handler (purge the session's settings and pending records).",
        |d, r| async move { on_session_deleted::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "approval::on_turn_completed",
        "Internal: harness::turn_completed handler (purge the turn's pending records).",
        |d, r| async move { on_turn_completed::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        "approval::sweep",
        "Internal: cron handler (expire pending records past expires_at).",
        |d, r| async move { sweep::handle(&d, r).await },
    );

    tracing::info!("all approval::* functions registered");
}
