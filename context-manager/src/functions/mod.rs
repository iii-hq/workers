//! The four `context::*` functions.
//!
//! Each `<verb>.rs` holds the request/response types (serde +
//! `schemars::JsonSchema`, so the SDK emits request/response schemas)
//! and a `pub async fn handle(deps, req)` that the registration closure
//! wraps. BDD scenarios call the same `handle` functions directly, so
//! engine-free tests exercise the exact production code path.

pub mod assemble;
pub mod compact;
pub mod count_tokens;
pub mod prune;

use std::future::Future;
use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, III};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::core::budget::{fallback_model, ResolvedModel};
use crate::error::ContextError;
use crate::ports::Deps;
use crate::types::ModelInput;

pub const ASSEMBLE_ID: &str = "context::assemble";
pub const ASSEMBLE_DESC: &str =
    "Build the model-ready context (system prompt + budgeted messages) from a history; \
     applies prune and/or compaction as needed to fit the model's usable token budget.";

pub const COMPACT_ID: &str = "context::compact";
pub const COMPACT_DESC: &str =
    "Summarise the head of a history into a single compaction summary, keeping a recent \
     tail verbatim. Transient: the caller persists the result.";

pub const PRUNE_ID: &str = "context::prune";
pub const PRUNE_DESC: &str =
    "Replace verbose function outputs with placeholders without summarising. \
     A cheaper first pass before compaction.";

pub const COUNT_TOKENS_ID: &str = "context::count_tokens";
pub const COUNT_TOKENS_DESC: &str =
    "Estimate token usage for a set of messages (+ optional invocation schemas / system \
     prompt) vs a model.";

/// Resolve model limits per the spec order: inline `limits` ->
/// `router::models::get` -> conservative fallback (when allowed).
pub(crate) async fn resolve_model(
    deps: &Deps,
    input: &ModelInput,
) -> Result<ResolvedModel, ContextError> {
    if let Some(limits) = input.limits {
        return Ok(ResolvedModel::from_inline(limits));
    }
    let reason = match deps
        .resolver
        .get_model(input.provider.as_deref(), &input.id)
        .await
    {
        Ok(Some(model)) => return Ok(ResolvedModel::from_router(&model)),
        Ok(None) => format!("model {} is not in the router catalog", input.id),
        Err(e) => format!("router unavailable: {e}"),
    };
    if deps.config.allow_fallback_limits {
        tracing::warn!(model = %input.id, %reason, "using conservative fallback limits");
        return Ok(fallback_model());
    }
    Err(ContextError::ModelUnresolved(reason))
}

/// Register one typed handler under `id`, mapping `ContextError` into
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
    Fut: Future<Output = Result<Resp, ContextError>> + Send + 'static,
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
    register(iii, deps, ASSEMBLE_ID, ASSEMBLE_DESC, |d, r| async move {
        assemble::handle(&d, r).await
    });
    register(iii, deps, COMPACT_ID, COMPACT_DESC, |d, r| async move {
        compact::handle(&d, r).await
    });
    register(iii, deps, PRUNE_ID, PRUNE_DESC, |d, r| async move {
        prune::handle(&d, r).await
    });
    register(
        iii,
        deps,
        COUNT_TOKENS_ID,
        COUNT_TOKENS_DESC,
        |d, r| async move { count_tokens::handle(&d, r).await },
    );

    tracing::info!("all context::* functions registered");
}

/// One function's complete agent-facing wire surface: id, registration
/// description, and the schemars-derived request/response schemas.
pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

/// Schema generation MUST mirror iii-sdk's internal `json_schema_for`
/// (`SchemaSettings::draft07()` on the handler's request/response
/// types): `RegisterFunction::new_async` auto-extracts schemas from the
/// SAME structs referenced here, with the same schemars 0.8 generator
/// settings, so a catalog snapshot pins exactly what registration
/// emits.
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

/// The full wire-surface catalog, in registration order. Golden-tested
/// in `tests/schemas.rs`; keep in lockstep with `register_all`.
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<assemble::AssembleRequest, assemble::AssembleResponse>(ASSEMBLE_ID, ASSEMBLE_DESC),
        spec::<compact::CompactRequest, compact::CompactResponse>(COMPACT_ID, COMPACT_DESC),
        spec::<prune::PruneRequest, prune::PruneResponse>(PRUNE_ID, PRUNE_DESC),
        spec::<count_tokens::CountTokensRequest, count_tokens::CountTokensResponse>(
            COUNT_TOKENS_ID,
            COUNT_TOKENS_DESC,
        ),
    ]
}
