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

use crate::config::WorkerConfig;
use crate::error::ApprovalError;
use crate::events::EventSink;
use crate::gate_config::SharedDefaults;

// ---------------------------------------------------------------------------
// Function ID + description constants — single source of truth consumed by
// both register_all and catalog().
// ---------------------------------------------------------------------------

pub const GATE_ID: &str = "approval::gate";
pub const GATE_DESC: &str = "pre_dispatch hook: evaluate the permission model and answer continue / deny / hold; writes the pending inbox record on hold. Called by the harness only.";

pub const RESOLVE_ID: &str = "approval::resolve";
pub const RESOLVE_DESC: &str = "Apply a human decision to a held call: release it for execution (allow) or deliver a denial (deny). Human/console-only.";

pub const LIST_PENDING_ID: &str = "approval::list-pending";
pub const LIST_PENDING_DESC: &str = "The pending inbox across sessions, with tenancy filters; the catch-up path for notification workers after a restart.";

pub const GET_PENDING_ID: &str = "approval::get-pending";
pub const GET_PENDING_DESC: &str = "Read one pending record; null when resolved or unknown.";

pub const SET_MODE_ID: &str = "approval::set-mode";
pub const SET_MODE_DESC: &str =
    "Set the session's permission mode (manual / auto / full). Human/console-only.";

pub const ADD_ALWAYS_ALLOW_ID: &str = "approval::add-always-allow";
pub const ADD_ALWAYS_ALLOW_DESC: &str =
    "Add a function to the session's auto-mode trust list (idempotent). Human/console-only.";

pub const REMOVE_ALWAYS_ALLOW_ID: &str = "approval::remove-always-allow";
pub const REMOVE_ALWAYS_ALLOW_DESC: &str = "Remove a function from the session's auto-mode trust list (no-op when absent). Human/console-only.";

pub const APPROVE_ALWAYS_ID: &str = "approval::approve-always";
pub const APPROVE_ALWAYS_DESC: &str =
    "Record a per-session 'approve always' grant (honoured in every mode). Human/console-only.";

pub const GET_SETTINGS_ID: &str = "approval::get-settings";
pub const GET_SETTINGS_DESC: &str = "Read the session's effective settings (stored record or configuration defaults); never writes.";

pub const CLEAR_SETTINGS_ID: &str = "approval::clear-settings";
pub const CLEAR_SETTINGS_DESC: &str =
    "Drop the session's stored settings record (revert to configuration defaults).";

pub const ON_CONFIG_CHANGE_ID: &str = "approval::on-config-change";
pub const ON_CONFIG_CHANGE_DESC: &str =
    "Internal: configuration trigger handler (reload deployment defaults).";

pub const ON_SESSION_DELETED_ID: &str = "approval::on-session-deleted";
pub const ON_SESSION_DELETED_DESC: &str =
    "Internal: session::deleted handler (purge the session's settings and pending records).";

pub const ON_TURN_COMPLETED_ID: &str = "approval::on-turn-completed";
pub const ON_TURN_COMPLETED_DESC: &str =
    "Internal: harness::turn-completed handler (purge the turn's pending records).";

pub const SWEEP_ID: &str = "approval::sweep";
pub const SWEEP_DESC: &str = "Internal: cron handler (expire pending records past expires_at).";

/// Everything a function handler needs.
pub struct Deps {
    pub iii: Arc<III>,
    pub sink: Arc<dyn EventSink>,
    pub defaults: SharedDefaults,
    pub cfg: Arc<WorkerConfig>,
}

/// Register one typed handler under `id`, mapping `ApprovalError` into
/// the wire error shape (`code: message`).
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
    register(iii, deps, GATE_ID, GATE_DESC, |d, r| async move {
        gate::handle(&d, r).await
    });
    register(iii, deps, RESOLVE_ID, RESOLVE_DESC, |d, r| async move {
        resolve::handle(&d, r).await
    });
    register(
        iii,
        deps,
        LIST_PENDING_ID,
        LIST_PENDING_DESC,
        |d, r| async move { list_pending::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        GET_PENDING_ID,
        GET_PENDING_DESC,
        |d, r| async move { get_pending::handle(&d, r).await },
    );
    register(iii, deps, SET_MODE_ID, SET_MODE_DESC, |d, r| async move {
        set_mode::handle(&d, r).await
    });
    register(
        iii,
        deps,
        ADD_ALWAYS_ALLOW_ID,
        ADD_ALWAYS_ALLOW_DESC,
        |d, r| async move { add_always_allow::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        REMOVE_ALWAYS_ALLOW_ID,
        REMOVE_ALWAYS_ALLOW_DESC,
        |d, r| async move { remove_always_allow::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        APPROVE_ALWAYS_ID,
        APPROVE_ALWAYS_DESC,
        |d, r| async move { approve_always::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        GET_SETTINGS_ID,
        GET_SETTINGS_DESC,
        |d, r| async move { get_settings::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        CLEAR_SETTINGS_ID,
        CLEAR_SETTINGS_DESC,
        |d, r| async move { clear_settings::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        ON_CONFIG_CHANGE_ID,
        ON_CONFIG_CHANGE_DESC,
        |d, r| async move { on_config_change::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        ON_SESSION_DELETED_ID,
        ON_SESSION_DELETED_DESC,
        |d, r| async move { on_session_deleted::handle(&d, r).await },
    );
    register(
        iii,
        deps,
        ON_TURN_COMPLETED_ID,
        ON_TURN_COMPLETED_DESC,
        |d, r| async move { on_turn_completed::handle(&d, r).await },
    );
    register(iii, deps, SWEEP_ID, SWEEP_DESC, |d, r| async move {
        sweep::handle(&d, r).await
    });

    tracing::info!("all approval::* functions registered");
}

// ---------------------------------------------------------------------------
// Wire-surface catalog — golden-tested in tests/schemas.rs.
// ---------------------------------------------------------------------------

/// One function's complete agent-facing wire surface: id, registration
/// description, and the schemars-derived request/response schemas.
pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

/// Schema generation MUST mirror iii-sdk's internal `json_schema_for`
/// (`SchemaSettings::draft07()` on the handler's request/response types):
/// `RegisterFunction::new_async` auto-extracts schemas from the SAME structs
/// referenced here, with the same schemars 0.8 generator settings, so a
/// catalog snapshot pins exactly what registration emits.
fn schema_of<T: schemars::JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req, Resp>(function_id: &'static str, description: &'static str) -> FunctionSpec
where
    Req: schemars::JsonSchema,
    Resp: schemars::JsonSchema,
{
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

/// The full wire-surface catalog, in registration order. Golden-tested in
/// `tests/schemas.rs`; keep in lockstep with `register_all`.
pub fn catalog() -> Vec<FunctionSpec> {
    use crate::types::{
        AlwaysAllowMutationRequest, ApproveAlwaysRequest, ClearSettingsRequest,
        ClearSettingsResponse, GetPendingRequest, GetPendingResponse, GetSettingsRequest,
        GetSettingsResponse, HookInput, HookOutput, ListPendingRequest, ListPendingResponse,
        ResolveRequest, ResolveResponse, SetModeRequest, SettingsResponse,
    };
    use on_config_change::ConfigChangeEvent;
    use on_session_deleted::SessionDeletedEvent;
    use on_turn_completed::TurnCompletedEvent;
    use serde_json::Value;

    vec![
        spec::<HookInput, HookOutput>(GATE_ID, GATE_DESC),
        spec::<ResolveRequest, ResolveResponse>(RESOLVE_ID, RESOLVE_DESC),
        spec::<ListPendingRequest, ListPendingResponse>(LIST_PENDING_ID, LIST_PENDING_DESC),
        spec::<GetPendingRequest, Option<GetPendingResponse>>(GET_PENDING_ID, GET_PENDING_DESC),
        spec::<SetModeRequest, SettingsResponse>(SET_MODE_ID, SET_MODE_DESC),
        spec::<AlwaysAllowMutationRequest, SettingsResponse>(
            ADD_ALWAYS_ALLOW_ID,
            ADD_ALWAYS_ALLOW_DESC,
        ),
        spec::<AlwaysAllowMutationRequest, SettingsResponse>(
            REMOVE_ALWAYS_ALLOW_ID,
            REMOVE_ALWAYS_ALLOW_DESC,
        ),
        spec::<ApproveAlwaysRequest, SettingsResponse>(APPROVE_ALWAYS_ID, APPROVE_ALWAYS_DESC),
        spec::<GetSettingsRequest, GetSettingsResponse>(GET_SETTINGS_ID, GET_SETTINGS_DESC),
        spec::<ClearSettingsRequest, ClearSettingsResponse>(CLEAR_SETTINGS_ID, CLEAR_SETTINGS_DESC),
        spec::<ConfigChangeEvent, Value>(ON_CONFIG_CHANGE_ID, ON_CONFIG_CHANGE_DESC),
        spec::<SessionDeletedEvent, Value>(ON_SESSION_DELETED_ID, ON_SESSION_DELETED_DESC),
        spec::<TurnCompletedEvent, Value>(ON_TURN_COMPLETED_ID, ON_TURN_COMPLETED_DESC),
        spec::<Value, Value>(SWEEP_ID, SWEEP_DESC),
    ]
}

// ---------------------------------------------------------------------------
// Trigger wire-surface catalog.
// ---------------------------------------------------------------------------

/// One trigger type's complete agent-facing wire surface: id, description,
/// and the schemars-derived payload schema.
pub struct TriggerSpec {
    pub trigger_id: &'static str,
    pub description: &'static str,
    pub payload_schema: schemars::schema::RootSchema,
}

/// The full trigger wire-surface catalog, in registration order. Golden-tested
/// in `tests/schemas.rs`; keep in lockstep with `events::register_trigger_types`.
pub fn trigger_catalog() -> Vec<TriggerSpec> {
    use crate::events::{PENDING_CREATED, PENDING_RESOLVED};
    use crate::types::{PendingApprovalRecord, PendingResolvedEvent};

    vec![
        TriggerSpec {
            trigger_id: PENDING_CREATED,
            description: crate::events::PENDING_CREATED_DESC,
            payload_schema: schema_of::<PendingApprovalRecord>(),
        },
        TriggerSpec {
            trigger_id: PENDING_RESOLVED,
            description: crate::events::PENDING_RESOLVED_DESC,
            payload_schema: schema_of::<PendingResolvedEvent>(),
        },
    ]
}
