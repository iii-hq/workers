//! The statically registered `code-runner::*` functions.
//!
//! Each `<verb>.rs` holds its typed request/response structs; the handler
//! bodies are thin wrappers over `RuntimeManager`, which is what the tests
//! drive directly.

pub mod eval;
pub mod inject_guidance;
pub mod register;
pub mod teardown;

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};

use crate::manager::RuntimeManager;

pub const EVAL_ID: &str = "code-runner::eval";
pub const EVAL_DESC: &str =
    "Run code in an isolated microVM. Pass lang (\"node\" or \"python\"). By default eval is \
     ONE-SHOT: it boots a fresh VM, runs code, returns the result, and destroys the VM — \
     nothing persists, no files, no installed packages, and the response carries no \
     runtime_id (there is nothing left to address). Pass keep: true to leave the VM running \
     instead: the response's runtime_id then addresses it, and is the capability \
     code-runner::teardown needs to stop it later. Pass runtime_id on a later call to reuse \
     that same VM (same filesystem, fresh interpreter process each time) — that runtime is \
     never auto-stopped, you own it until you tear it down or its idle TTL reaps it, and a \
     reaped reuse fails with code-runner::expired (retry without runtime_id to boot a fresh \
     one). network: true asks for outbound network so npm/pip installs work, but only a \
     runtime you already created with network can honor it (pass its runtime_id) — neither a \
     one-shot eval nor keep: true can create a networked VM, so network: true without an \
     existing runtime_id is refused, not silently ignored. stdout, stderr and exit_code come \
     back verbatim — a failing script is a response, not an error.";

pub const TEARDOWN_ID: &str = "code-runner::teardown";
pub const TEARDOWN_DESC: &str =
    "Destroy a runtime: unregister every bus function it registered, stop its microVM(s), and \
     free the slot(s). Pass exactly one of runtime_id (a kept eval's runtime, from \
     code-runner::eval keep=true) or namespace (a register_function namespace, e.g. \"app\" for \
     ids like app::greet) — never both, never neither.";

pub const REGISTER_ID: &str = "code-runner::register_function";
pub const REGISTER_DESC: &str =
    "Publish a bus function whose handler executes inside a microVM. No runtime_id needed: \
     code-runner keeps one persistent runtime per namespace (the segment of function_id before \
     `::`) and language — the first registration in a namespace boots it, later ones in the \
     same namespace and lang reuse it automatically. `source` must DEFINE handler(payload) in \
     `lang` — `export function handler(payload) {...}` (node) or `def handler(payload): ...` \
     (python); each call runs it in a fresh interpreter process with the trigger payload and \
     returns its JSON-serialized result. The first registered id in a namespace claims it; \
     later ids must share both the namespace and its lang. `description` is what \
     engine::functions::info shows a caller — write one. Functions stop resolving when their \
     namespace is torn down (code-runner::teardown namespace=...) or its runtime is reaped for \
     idleness.";

/// Every id this worker registers on its own client, in registration order.
/// `register_all` asserts it registered exactly this list, and the schema
/// test pins `catalog()` to it — the two hand-maintained lists must not
/// drift apart.
pub const STATIC_IDS: &[&str] = &[
    EVAL_ID,
    TEARDOWN_ID,
    REGISTER_ID,
    inject_guidance::GUIDANCE_HOOK_ID,
];

pub fn register_all(iii: &Arc<IIIClient>, manager: &Arc<RuntimeManager>) {
    // Seed the local claims registry with this worker's own ids BEFORE
    // registering anything, so `RuntimeManager::register`'s reservation
    // check refuses a caller-supplied `code-runner::*` id from the moment
    // this function starts, rather than depending on the
    // `engine::functions::info` probe (a network round trip) to catch it.
    manager.seed_static_ids(STATIC_IDS);

    let mut registered: Vec<&str> = Vec::new();

    let m = manager.clone();
    registered.push(EVAL_ID);
    iii.register_function(
        EVAL_ID,
        RegisterFunction::new_async(move |req: eval::EvalRequest| {
            let m = m.clone();
            async move { m.eval(req).await.map_err(Error::from) }
        })
        .description(EVAL_DESC),
    );

    let m = manager.clone();
    registered.push(TEARDOWN_ID);
    iii.register_function(
        TEARDOWN_ID,
        RegisterFunction::new_async(move |req: teardown::TeardownRequest| {
            let m = m.clone();
            async move { m.teardown(req).await.map_err(Error::from) }
        })
        .description(TEARDOWN_DESC),
    );

    let m = manager.clone();
    registered.push(REGISTER_ID);
    iii.register_function(
        REGISTER_ID,
        RegisterFunction::new_async(move |req: register::RegisterRequest| {
            let m = m.clone();
            async move { m.register(req).await.map_err(Error::from) }
        })
        .description(REGISTER_DESC),
    );

    registered.push(inject_guidance::GUIDANCE_HOOK_ID);
    iii.register_function(
        inject_guidance::GUIDANCE_HOOK_ID,
        RegisterFunction::new_async(move |event: inject_guidance::PreGenerateEvent| async move {
            inject_guidance::handle(event).await
        })
        .description(inject_guidance::GUIDANCE_HOOK_DESC)
        // The harness calls this, never an agent; keep it out of the
        // callable catalog agents browse.
        .metadata(serde_json::json!({ "internal": true })),
    );

    assert_eq!(
        registered, STATIC_IDS,
        "register_all must register exactly STATIC_IDS — the lists must not drift"
    );

    tracing::info!("code-runner functions registered");
}

/// Bind the `pre_generate` hook so the guidance reaches the agent's system
/// prompt while this worker is connected. `on_error: fail_open` is
/// MANDATORY — `pre_generate` defaults to fail-CLOSED, and a missing
/// guidance line must never abort an agent's turn.
pub fn setup_harness_hooks(iii: &Arc<IIIClient>) {
    match iii.register_trigger(iii_sdk::protocol::RegisterTriggerInput {
        trigger_type: "harness::hook::pre-generate".to_string(),
        function_id: inject_guidance::GUIDANCE_HOOK_ID.to_string(),
        config: serde_json::json!({ "on_error": "fail_open" }),
        metadata: None,
    }) {
        Ok(_) => tracing::info!("code-runner pre-generate hook bound (guidance injection active)"),
        Err(e) => tracing::warn!(error = %e, "guidance hook binding failed; continuing without it"),
    }
}

pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

/// Schema generation MUST mirror iii-sdk's internal `json_schema_for`
/// (`SchemaSettings::draft07()` on the handler's request/response types), so
/// a catalog snapshot pins exactly what registration emits.
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

/// Every statically registered function, in registration order.
pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<eval::EvalRequest, eval::EvalResponse>(EVAL_ID, EVAL_DESC),
        spec::<teardown::TeardownRequest, teardown::TeardownResponse>(TEARDOWN_ID, TEARDOWN_DESC),
        spec::<register::RegisterRequest, register::RegisterResponse>(REGISTER_ID, REGISTER_DESC),
        spec::<inject_guidance::PreGenerateEvent, inject_guidance::PreGenerateResponse>(
            inject_guidance::GUIDANCE_HOOK_ID,
            inject_guidance::GUIDANCE_HOOK_DESC,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CodeRunnerConfig;
    use crate::engine::IIIEngine;
    use iii_sdk::IIIClient;

    /// Before `code-runner` had a `[[bin]]` target, nothing ever called
    /// `register_all` — `main` is the only caller, and the `--manifest`
    /// smoke test returns before it runs. Its trailing
    /// `assert_eq!(registered, STATIC_IDS, ...)` is real protection: a
    /// function registered but left out of `STATIC_IDS` could be claimed by
    /// tenant code and hit iii-sdk's documented panic-on-duplicate-id. Drive
    /// it here so that assertion is exercised by the suite, not first by
    /// production. `IIIClient::new` only builds local state — no network —
    /// same trick node-engine's own `engine.rs` tests rely on.
    #[test]
    fn register_all_registers_exactly_static_ids() {
        let iii = Arc::new(IIIClient::new("ws://127.0.0.1:1"));
        let engine = Arc::new(IIIEngine::new(iii.clone()));
        let manager = RuntimeManager::new(Arc::new(CodeRunnerConfig::default()), engine);
        register_all(&iii, &manager);
    }
}
