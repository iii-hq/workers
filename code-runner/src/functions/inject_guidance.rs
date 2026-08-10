//! `code-runner::inject-guidance` — a `pre_generate` hook that contributes the
//! `code-runner::*` usage guidance to the agent's system prompt, ONLY while this
//! worker is connected. The binding dies with the worker, so the guidance is
//! presence-gated for free: a deployment without code-runner never pays for it,
//! and the text is never hand-duplicated into a static harness prompt.
//!
//! Mirrors `web/src/functions/inject_guidance.rs`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const GUIDANCE_HOOK_ID: &str = "code-runner::inject-guidance";
pub const GUIDANCE_HOOK_DESC: &str =
    "Internal pre_generate hook: appends code-runner usage guidance to the agent system \
     prompt. Bound to harness::hook::pre-generate at worker startup; not called directly.";

/// The single canonical copy of the code-runner usage guidance. Pure USAGE
/// guidance: the hook only fires while this worker is present, so it carries no
/// "look for it / install it" discovery text.
const NODE_ENGINE_GUIDANCE: &str = "To run JavaScript — computing something, reshaping data, or exposing a piece of logic other workers can call — use `code-runner::*` rather than a throwaway script through `shell::exec`. `code-runner::run` evaluates code in a V8 isolate: your code is wrapped in an async IIFE, so `return` sets the result and top-level `await` works. By default `run` is ONE-SHOT — it creates a runtime, evaluates the code, and disposes the runtime before responding, so the response carries no `runtime_id`. Pass `keep: true` to leave the runtime running instead: the response's `runtime_id` then addresses it, and is the capability `code-runner::teardown` needs to stop it later — treat `runtime_id` as a secret. Pass `runtime_id` on a later call to reuse that same isolate, sharing its globals and registrations; a runtime you supplied `runtime_id` for is never disposed by that call, you own it until you tear it down. Inside evaluated code the only host surface is the `iii` global: `await iii.trigger({function_id, payload, action?, timeout?})` calls any engine function; `iii.registerFunction(functionId, handler, options?)` publishes a live function for the life of this runtime and returns `{unregister()}`; `iii.registerTrigger(input)`, `iii.registerTriggerType(triggerType, handler)` and `iii.unregisterTriggerType(triggerType)` are also available; `iii.namespace` is the prefix this runtime may register under; `iii.shutdown()` throws by design, pointing you at `code-runner::teardown` instead — the connection belongs to the worker, shared by every runtime on it; `console` output comes back in `logs`. To publish a JS function on the bus without hand-authoring a run payload, use `code-runner::register_function`: one function per call, `{function_id: \"my-app::greet\", source: \"export function handler(p) { return p.n * 2 }\", description: \"what it does\"}`. `source` must DEFINE `handler(payload)` — `export function handler(p) {...}` and `const handler = ...` both work (`export` is only recognised as the very FIRST token of `source`); a bare expression like `\"(p) => p.n * 2\"` is refused, since nothing is bound to that name. The segment before the first `::` in `function_id` is the namespace: code-runner keeps one persistent runtime per namespace, created by the first registration and reused by every later one, as an implementation detail you never see or manage — there is no `runtime_id` for it to hand back. Always write a `description`: it is what `engine::functions::info` shows the next caller — and pass `request_format`/`response_format` (JSON Schema objects) when callers must know the payload shape; `engine::functions::info` then shows the real schemas instead of \"any\". An id another runtime already holds is refused as `code-runner::invalid_request`, its message saying the id is already registered. Registrations live exactly as long as their runtime: call `code-runner::teardown` with `runtime_id` or `namespace` (exactly one) when the work is done — everything that runtime registered stops resolving the moment it is torn down or swept for being idle, so do not register a function and walk away expecting it to answer later. This is plain V8, NOT Node: there is no npm, no `require`, no `fs`, no `fetch`, and no Node builtins — not even `global`; state that must survive later calls on a kept runtime goes on `globalThis`. Fetch exact request shapes via `engine::functions::info` before the first call.";

/// The slice of the `pre_generate` hook envelope we read (lenient: ignores every
/// other field the harness sends). The harness nests the live generation context
/// under `generate`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PreGenerateEvent {
    #[serde(default)]
    pub generate: GenerateContext,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GenerateContext {
    /// The system prompt assembled so far (base + any prior hook's mutation).
    #[serde(default)]
    pub system_prompt: String,
}

/// Hook envelope returned to the harness: the mutations to apply to the
/// generation.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PreGenerateResponse {
    pub mutations: PreGenerateMutations,
}

/// The harness applies `system_prompt` only when the key is present, so `None`
/// serializes to an empty object: the safe no-op that preserves the harness's
/// assembled prompt.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct PreGenerateMutations {
    /// Full replacement system prompt (base + appended guidance). The harness
    /// overwrites, it does not merge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

/// Build the `pre_generate` mutations for a given base prompt. Pure, so it is
/// unit-testable.
///
/// Returns NO `system_prompt` when `base` is empty. A missing or renamed
/// `generate.system_prompt` deserializes to `""` (schema drift), and a fail-open
/// hook must PRESERVE the harness's assembled prompt, never replace it with the
/// guidance alone.
fn mutations_for(base: &str) -> PreGenerateMutations {
    if base.is_empty() {
        PreGenerateMutations::default()
    } else {
        PreGenerateMutations {
            system_prompt: Some(format!("{base}\n\n{NODE_ENGINE_GUIDANCE}")),
        }
    }
}

/// `pre_generate` hook entrypoint. Bound `fail_open`, so an error here never
/// blocks a turn.
pub async fn handle(
    event: PreGenerateEvent,
) -> Result<PreGenerateResponse, iii_sdk::errors::Error> {
    Ok(PreGenerateResponse {
        mutations: mutations_for(&event.generate.system_prompt),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_guidance_after_a_real_base() {
        let m = mutations_for("BASE PROMPT");
        let sp = m
            .system_prompt
            .expect("a non-empty base yields a system_prompt mutation");
        assert!(
            sp.starts_with("BASE PROMPT\n\n"),
            "the base prompt must be preserved, guidance appended after it"
        );
        assert!(sp.contains("code-runner::run"), "guidance content present");
    }

    #[test]
    fn empty_base_emits_no_system_prompt_mutation() {
        // A missing/malformed hook payload (system_prompt absent → "") must
        // PRESERVE the harness prompt: emit no system_prompt key, rather than
        // replacing the whole prompt with the guidance alone. The wire shape
        // must stay `{"mutations": {}}`.
        let wire = serde_json::to_value(PreGenerateResponse {
            mutations: mutations_for(""),
        })
        .expect("response serializes");
        assert_eq!(wire, serde_json::json!({ "mutations": {} }));
    }

    /// Mirrors the registry publish gate: the derived response schema must
    /// carry a schema-defining keyword, not the permissive AnyValue schema.
    #[test]
    fn response_schema_passes_the_publish_typed_gate() {
        let schema = schemars::r#gen::SchemaSettings::draft07()
            .into_generator()
            .into_root_schema_for::<PreGenerateResponse>();
        let value = serde_json::to_value(schema).expect("schema serializes");
        let obj = value.as_object().expect("schema is an object");
        assert!(
            ["type", "properties", "$ref"]
                .iter()
                .any(|k| obj.contains_key(*k)),
            "PreGenerateResponse schema is untyped: {value}"
        );
    }

    #[test]
    fn guidance_covers_this_worker_s_surface() {
        // Each needle is a fact an agent gets wrong without the guidance: the
        // three function ids, that it is NOT Node, that the description field
        // exists, and that runtime_id is a capability.
        for needle in [
            "code-runner::run",
            "code-runner::register_function",
            "code-runner::teardown",
            "NOT Node",
            "no npm",
            "engine::functions::info",
            "treat `runtime_id` as a secret",
            "keep: true",
            "iii.namespace",
            "stops resolving the moment it is torn down",
            // The two an agent gets wrong without being told, each one
            // observed rather than imagined: `source` must bind a name
            // (not just evaluate to a function), and `shutdown` throwing
            // rather than silently no-op-ing.
            "must DEFINE `handler(payload)`",
            "iii.shutdown()",
            "throws by design",
            // Observed: an agent wrote Node-idiom `global.counter` and got
            // `ReferenceError: global is not defined` — the `lang: "node"`
            // name primes the Node mental model, so the guidance must name
            // the standard alias.
            "goes on `globalThis`",
            // The id-clash sentence must quote the REAL wire code: this
            // worker's taxonomy has no `id_taken` (error.rs), and guidance
            // inventing one taught agents to match a phantom code.
            "code-runner::invalid_request",
            // Without this an agent has no way to know the catalog can show
            // real schemas — the register card and functions::info both said
            // "any" for every dynamic registration until these fields.
            "request_format",
        ] {
            assert!(
                NODE_ENGINE_GUIDANCE.contains(needle),
                "guidance is missing: {needle}"
            );
        }
    }

    /// Guidance is the first thing an agent reads about this worker. Text
    /// describing a removed function is worse than no text: it produces calls
    /// that cannot succeed.
    #[test]
    fn guidance_describes_only_functions_that_exist() {
        for gone in [
            "register_worker",
            "eject",
            "code-runner::eval",
            "callFunction",
            // Not a wire code (error.rs) — an id clash is re-coded to
            // `code-runner::invalid_request`, and guidance quoting `id_taken`
            // taught agents to match a code that never arrives.
            "id_taken",
        ] {
            assert!(
                !NODE_ENGINE_GUIDANCE.contains(gone),
                "guidance still mentions {gone}"
            );
        }
        for present in [
            "code-runner::run",
            "code-runner::register_function",
            "iii.trigger",
            "keep",
        ] {
            assert!(
                NODE_ENGINE_GUIDANCE.contains(present),
                "guidance never mentions {present}"
            );
        }
    }
}
