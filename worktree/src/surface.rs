//! Wire-surface catalog for every function this worker registers — the
//! single source of truth for each function's id and schemars-derived
//! request/response schemas. Golden-tested in `tests/schemas.rs`; keep in
//! lockstep with the boot registration order in `main.rs`
//! (`functions::register_all`, then the configuration handler).
//!
//! Schema generation MUST mirror iii-sdk's internal `json_schema_for`
//! (`SchemaSettings::draft07()` on the handler's request/response types) so
//! a catalog snapshot pins exactly what registration emits.

use crate::configuration::{OnConfigChangeEvent, OnConfigChangeResponse};
use crate::functions::{
    claim, create, get, land, land_step, list, prune, release, remove, status, validate,
};

/// One function's wire surface: id plus the schemars-derived
/// request/response schemas.
pub struct FunctionSpec {
    pub function_id: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

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

/// The full wire-surface catalog, in registration order.
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<create::Request, create::Response>("worktree::create"),
        spec::<list::Request, list::Response>("worktree::list"),
        spec::<get::Request, get::Response>("worktree::get"),
        spec::<validate::Request, validate::Response>("worktree::validate"),
        spec::<claim::Request, claim::Response>("worktree::claim"),
        spec::<release::Request, release::Response>("worktree::release"),
        spec::<status::Request, status::Response>("worktree::status"),
        spec::<remove::Request, remove::Response>("worktree::remove"),
        spec::<prune::Request, prune::Response>("worktree::prune"),
        spec::<land::Request, land::Response>("worktree::land"),
        spec::<land_step::Request, land_step::Response>("worktree::land-step"),
        spec::<OnConfigChangeEvent, OnConfigChangeResponse>("worktree::on-config-change"),
    ]
}
