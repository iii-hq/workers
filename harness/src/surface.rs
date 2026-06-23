//! Wire-surface catalog: the single source of truth for each registered
//! function's id and schemars-derived request/response schemas. Golden-tested
//! in `tests/schemas.rs`; keep in lockstep with `functions::register_all`.
//!
//! Internal trigger-bound handlers (`harness::sweep-pending`,
//! `harness::on-config-change`) are intentionally excluded — they are not part
//! of the agent-facing surface.

use crate::functions::{
    continue_turn::{ContinueRequest, ContinueResponse},
    function_resolve::{FunctionResolveRequest, FunctionResolveResponse},
    function_trigger::{FunctionTriggerRequest, FunctionTriggerResponse},
    send::{SendRequest, SendResponse},
    spawn::{SpawnRequest, SpawnResponse},
    status::{StatusReport, StatusRequest},
    stop::{StopRequest, StopResponse},
};
use crate::functions::{
    CONTINUE_ID, FUNCTION_RESOLVE_ID, FUNCTION_TRIGGER_ID, SEND_ID, SPAWN_ID, STATUS_ID, STOP_ID,
    TURN_ID,
};
use crate::turn_loop::{TurnStepPayload, TurnStepResult};

pub struct FunctionSpec {
    pub function_id: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

/// Mirror iii-sdk's internal `json_schema_for` (`SchemaSettings::draft07()`)
/// so a catalog snapshot pins exactly what registration emits.
fn schema_of<T: schemars::JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req, Resp>(function_id: &'static str) -> FunctionSpec
where
    Req: schemars::JsonSchema,
    Resp: schemars::JsonSchema,
{
    FunctionSpec {
        function_id,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

/// Draft-07 schema as a wire `Value`, for explicit `.request_format` /
/// `.response_format` overrides on registrations whose handler signature
/// cannot drive schemars auto-extraction (e.g. lossless `Value` payloads).
pub fn schema_value<T: schemars::JsonSchema>() -> serde_json::Value {
    serde_json::to_value(schema_of::<T>()).expect("schema serializes to JSON")
}

/// The full agent-facing wire-surface catalog, in registration order.
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<SendRequest, SendResponse>(SEND_ID),
        spec::<SpawnRequest, SpawnResponse>(SPAWN_ID),
        spec::<TurnStepPayload, TurnStepResult>(TURN_ID),
        spec::<FunctionTriggerRequest, FunctionTriggerResponse>(FUNCTION_TRIGGER_ID),
        spec::<FunctionResolveRequest, FunctionResolveResponse>(FUNCTION_RESOLVE_ID),
        spec::<StopRequest, StopResponse>(STOP_ID),
        spec::<StatusRequest, Option<StatusReport>>(STATUS_ID),
        spec::<ContinueRequest, ContinueResponse>(CONTINUE_ID),
    ]
}
