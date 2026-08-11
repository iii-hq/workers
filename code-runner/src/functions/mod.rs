//! The statically registered `code-runner::*` functions.
//!
//! Each `<verb>.rs` holds its typed request/response structs; the handler
//! bodies are thin wrappers over `Manager`, which is what the tests drive
//! directly.

pub mod inject_guidance;
pub mod register;
pub mod run;
pub mod teardown;

use std::sync::Arc;

use iii_sdk::{IIIClient, RegisterFunction};

use crate::manager::Manager;

pub const RUN_ID: &str = "code-runner::run";
/// Contract essentials only — the injected harness guidance
/// (`inject_guidance`) teaches the full surface, and every extra byte here
/// is paid on each catalog read.
pub const RUN_DESC: &str =
    "Run code in an in-process sandbox — lang \"node\" or \"python\"; no network, no host \
     filesystem, no package installs. One-shot by default; keep: true returns a runtime_id, \
     and passing runtime_id later reuses that runtime (same globals and scratch) until \
     code-runner::teardown or the idle TTL reaps it (then: code-runner::expired — retry \
     without runtime_id). Code gets an `iii` global for bus calls. stdout/stderr/exit_code \
     come back verbatim — a failing script is a response, not an error — and `result` \
     carries the returned value: node code is a FUNCTION BODY (`return x`), python a MODULE \
     (assign `result = x`).";

pub const TEARDOWN_ID: &str = "code-runner::teardown";
pub const TEARDOWN_DESC: &str = "Destroy a runtime: unregister every bus function it registered, \
     kill it, and free its slot. Pass exactly one of runtime_id (a kept run's runtime, from \
     code-runner::run keep=true) or namespace (a register_function namespace, e.g. \"app\" for \
     ids like app::greet) — never both, never neither.";

pub const REGISTER_ID: &str = "code-runner::register_function";
pub const REGISTER_DESC: &str = "Publish a bus function whose handler runs in a sandbox. \
     `source` must DEFINE handler(payload) in `lang`; one persistent runtime per namespace \
     (function_id before `::`) is booted by the first registration and shared by later ones \
     of the same lang. Write a `description` for callers. Optional `request_format` / \
     `response_format` (JSON Schema objects, max 16 KiB) are shown by engine::functions::info \
     but NOT enforced at call time — validate inside the handler. Registrations stop \
     resolving when the namespace is torn down or its runtime reaped for idleness.";

/// Every id this worker registers on its own client, in registration order.
///
/// The order is `run, teardown, register_function, inject-guidance`, matching
/// sandbox-code-runner's own — including the odd teardown-before-register
/// placement. `catalog()` and the golden schema snapshots agree with it, so
/// do not "fix" the ordering.
pub const STATIC_IDS: &[&str] = &[
    RUN_ID,
    TEARDOWN_ID,
    REGISTER_ID,
    inject_guidance::GUIDANCE_HOOK_ID,
];

/// Every id this worker owns, for seeding the runtime manager's id registry.
///
/// STRICTLY LARGER than `STATIC_IDS`, which is only what `register_all`
/// registers: the console UI's content function is published later, by
/// `crate::ui::register`, and it must be seeded all the same.
///
/// Why it matters. The registry is what stops a caller claiming an id the
/// worker already holds. `register_function` lets a caller pick ANY namespace,
/// including `code-runner::`, so an unseeded worker id can be claimed — and the
/// claim then reaches `iii_sdk`'s `register_function` on an id that is already
/// registered, whose duplicate-id panic ABORTS THE PROCESS. Verified
/// reachable: before `ui-content` was seeded,
/// `register_function("code-runner::ui-content", …)` returned
/// `registered: true`.
pub fn seeded_ids() -> Vec<&'static str> {
    let mut ids = STATIC_IDS.to_vec();
    ids.push(crate::ui::CONTENT_FUNCTION_ID);
    // Registered by `configuration::register_config_trigger`, after this seed
    // exists — same reasoning as `ui-content` above.
    ids.push(crate::configuration::CONFIG_FN_ID);
    ids
}

pub fn register_all(iii: &Arc<IIIClient>, manager: &Arc<Manager>) {
    let mut registered: Vec<&str> = Vec::new();

    let m = manager.clone();
    registered.push(RUN_ID);
    iii.register_function(
        RUN_ID,
        RegisterFunction::new_async(move |req: run::RunRequest| {
            let m = m.clone();
            async move { m.run(req).await.map_err(iii_sdk::errors::Error::from) }
        })
        .description(RUN_DESC),
    );

    let m = manager.clone();
    registered.push(TEARDOWN_ID);
    iii.register_function(
        TEARDOWN_ID,
        RegisterFunction::new_async(move |req: teardown::TeardownRequest| {
            let m = m.clone();
            async move { m.teardown(req).await.map_err(iii_sdk::errors::Error::from) }
        })
        .description(TEARDOWN_DESC),
    );

    let m = manager.clone();
    registered.push(REGISTER_ID);
    iii.register_function(
        REGISTER_ID,
        RegisterFunction::new_async(move |req: register::RegisterRequest| {
            let m = m.clone();
            async move { m.register(req).await.map_err(iii_sdk::errors::Error::from) }
        })
        .description(REGISTER_DESC),
    );

    let cfg = manager.shared_config();
    registered.push(inject_guidance::GUIDANCE_HOOK_ID);
    iii.register_function(
        inject_guidance::GUIDANCE_HOOK_ID,
        RegisterFunction::new_async(move |event: inject_guidance::PreGenerateEvent| {
            let enabled = cfg.load().inject_guidance;
            async move { inject_guidance::handle(event, enabled).await }
        })
        .description(inject_guidance::GUIDANCE_HOOK_DESC)
        // The harness calls this, never an agent; it must not show up as a
        // callable option alongside run/register_function/teardown.
        .metadata(serde_json::json!({ "internal": true })),
    );

    assert_eq!(
        registered, STATIC_IDS,
        "register_all must register exactly STATIC_IDS, in order: the id registry is seeded \
         from that list, and an unseeded registration lets a runtime claim a worker id"
    );

    tracing::info!("code-runner functions registered");
}

/// Bind the `pre_generate` hook so the guidance reaches the agent's system
/// prompt while this worker is connected.
///
/// `on_error: fail_open` is MANDATORY. `pre_generate` defaults to
/// fail-CLOSED, so an error or timeout here would abort the agent's turn; a
/// missing guidance line must never do that.
pub fn setup_harness_hooks(iii: &Arc<IIIClient>) {
    match iii.register_trigger(iii_sdk::protocol::RegisterTriggerInput {
        trigger_type: "harness::hook::pre-generate".to_string(),
        function_id: inject_guidance::GUIDANCE_HOOK_ID.to_string(),
        config: serde_json::json!({ "on_error": "fail_open" }),
        metadata: None,
    }) {
        Ok(_) => tracing::info!("code-runner pre-generate hook bound"),
        Err(e) => tracing::warn!(error = %e, "guidance hook binding failed; continuing without it"),
    }
}

pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

/// MUST mirror iii-sdk's internal `json_schema_for` (draft07) so a catalog
/// snapshot pins exactly what registration emits.
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

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<run::RunRequest, run::RunResponse>(RUN_ID, RUN_DESC),
        spec::<teardown::TeardownRequest, teardown::TeardownResponse>(TEARDOWN_ID, TEARDOWN_DESC),
        spec::<register::RegisterRequest, register::RegisterResponse>(REGISTER_ID, REGISTER_DESC),
        spec::<inject_guidance::PreGenerateEvent, inject_guidance::PreGenerateResponse>(
            inject_guidance::GUIDANCE_HOOK_ID,
            inject_guidance::GUIDANCE_HOOK_DESC,
        ),
    ]
}
