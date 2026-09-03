//! The statically registered `sandbox-code-runner::*` functions.
//!
//! Each `<verb>.rs` holds its typed request/response structs; the handler
//! bodies are thin wrappers over `RuntimeManager`, which is what the tests
//! drive directly.

pub mod inject_guidance;
pub mod list_runtimes;
pub mod register;
pub mod run;
pub mod teardown;

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};

use crate::manager::RuntimeManager;

pub const RUN_ID: &str = "sandbox-code-runner::run";
pub const RUN_DESC: &str =
    "Run code in an isolated microVM with outbound network. One-shot by default: nothing \
     persists and no runtime_id is returned. keep: true keeps the VM and returns its \
     runtime_id to reuse until sandbox-code-runner::teardown or the idle TTL reaps it. A \
     failing script is a response, not an error.";

pub const TEARDOWN_ID: &str = "sandbox-code-runner::teardown";
pub const TEARDOWN_DESC: &str =
    "Destroy a runtime: unregister its bus functions, stop its microVM(s) and free the slot. \
     Pass exactly one of runtime_id (from a keep: true run) or namespace (a register_function \
     namespace, e.g. \"app\" for app::greet).";

pub const REGISTER_ID: &str = "sandbox-code-runner::register_function";
pub const REGISTER_DESC: &str =
    "Publish a bus function whose handler runs in a microVM; no runtime_id needed. source \
     must define handler(payload) in lang; each call runs it and returns its JSON result. The \
     namespace (function_id before `::`) shares one runtime per lang; its functions stop \
     resolving once torn down or idle-reaped.";

pub const LIST_RUNTIMES_ID: &str = "sandbox-code-runner::list_runtimes";
pub const LIST_RUNTIMES_DESC: &str =
    "List every runtime this worker holds (kept runs and register_function namespaces), \
     newest first, with runtime_id, lang, sandbox_id, created_at_ms and registered function \
     ids. One-shot runs never appear.";

/// Every id this worker registers on its own client, in registration order.
/// `register_all` asserts it registered exactly this list, and the schema
/// test pins `catalog()` to it — the two hand-maintained lists must not
/// drift apart.
pub const STATIC_IDS: &[&str] = &[
    RUN_ID,
    TEARDOWN_ID,
    REGISTER_ID,
    LIST_RUNTIMES_ID,
    inject_guidance::GUIDANCE_HOOK_ID,
];

/// Every id this worker owns, for seeding the runtime manager's id registry.
///
/// STRICTLY LARGER than `STATIC_IDS`, which is only what `register_all`
/// registers: the console UI's content function is published later by
/// `crate::ui::register`, and the config reload hook later still by
/// `configuration::register_config_trigger` — after two awaited
/// configuration RPCs, a window in which a guest `register_function` could
/// otherwise claim the id and drive the SDK's duplicate-id panic when the
/// worker's own registration lands (aborting the whole process). Same
/// pattern and reasoning as code-runner's `seeded_ids`.
pub fn seeded_ids() -> Vec<&'static str> {
    let mut ids = STATIC_IDS.to_vec();
    ids.push(crate::ui::CONTENT_FUNCTION_ID);
    ids.push(crate::configuration::CONFIG_FN_ID);
    ids
}

pub fn register_all(iii: &Arc<IIIClient>, manager: &Arc<RuntimeManager>) {
    // Seed the local claims registry with EVERY id this worker will register
    // (`seeded_ids`, not `STATIC_IDS`: the ui-content and on-config-change
    // registrations land after guest-facing `register` is live) BEFORE
    // registering anything, so `RuntimeManager::register`'s reservation
    // check refuses a caller-supplied `sandbox-code-runner::*` id from the
    // moment this function starts, rather than depending on the
    // `engine::functions::info` probe (a network round trip) to catch it.
    manager.seed_static_ids(&seeded_ids());

    let mut registered: Vec<&str> = Vec::new();

    let m = manager.clone();
    registered.push(RUN_ID);
    iii.register_function(
        RUN_ID,
        RegisterFunction::new_async(move |req: run::RunRequest| {
            let m = m.clone();
            async move { m.run(req).await.map_err(Error::from) }
        })
        .description(RUN_DESC),
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

    let m = manager.clone();
    registered.push(LIST_RUNTIMES_ID);
    iii.register_function(
        LIST_RUNTIMES_ID,
        RegisterFunction::new_async(move |_req: list_runtimes::ListRuntimesRequest| {
            let m = m.clone();
            async move { Ok::<_, Error>(m.list_runtimes().await) }
        })
        .description(LIST_RUNTIMES_DESC),
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

    tracing::info!("sandbox-code-runner functions registered");
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
        spec::<run::RunRequest, run::RunResponse>(RUN_ID, RUN_DESC),
        spec::<teardown::TeardownRequest, teardown::TeardownResponse>(TEARDOWN_ID, TEARDOWN_DESC),
        spec::<register::RegisterRequest, register::RegisterResponse>(REGISTER_ID, REGISTER_DESC),
        spec::<list_runtimes::ListRuntimesRequest, list_runtimes::ListRuntimesResponse>(
            LIST_RUNTIMES_ID,
            LIST_RUNTIMES_DESC,
        ),
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

    /// Before `sandbox-code-runner` had a `[[bin]]` target, nothing ever
    /// called `register_all` — `main` is the only caller, and the
    /// `--manifest` smoke test returns before it runs. Its trailing
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
        let manager = RuntimeManager::new(
            Arc::new(CodeRunnerConfig::default()),
            engine,
            "ws://127.0.0.1:1",
        );
        register_all(&iii, &manager);
    }

    /// The ids registered OUTSIDE `register_all` (ui-content by
    /// `ui::register`, on-config-change by `register_config_trigger`) must be
    /// reserved by the seed all the same: an unseeded late registration is
    /// claimable by a guest during the boot window, and the claim ends in the
    /// SDK's duplicate-id process abort when the worker's own registration
    /// lands.
    #[test]
    fn seeded_ids_cover_the_late_registrations() {
        let ids = seeded_ids();
        for id in STATIC_IDS {
            assert!(ids.contains(id), "seed lost a static id: {id}");
        }
        assert!(ids.contains(&crate::ui::CONTENT_FUNCTION_ID));
        assert!(ids.contains(&crate::configuration::CONFIG_FN_ID));
    }
}
