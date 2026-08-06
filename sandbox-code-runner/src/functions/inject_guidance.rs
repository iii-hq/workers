//! `sandbox-code-runner::inject-guidance` — a `pre_generate` hook that
//! contributes the `sandbox-code-runner::*` usage guidance to the agent's
//! system prompt, ONLY while this worker is connected. The binding dies with
//! the worker, so the guidance is presence-gated for free: a deployment
//! without sandbox-code-runner never pays for it, and the text is never
//! hand-duplicated into a static harness prompt.
//!
//! Mirrors `web/src/functions/inject_guidance.rs`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const GUIDANCE_HOOK_ID: &str = "sandbox-code-runner::inject-guidance";
pub const GUIDANCE_HOOK_DESC: &str =
    "Internal pre_generate hook: appends sandbox-code-runner usage guidance to the agent \
     system prompt. Bound to harness::hook::pre-generate at worker startup; not called \
     directly.";

/// The single canonical copy of the sandbox-code-runner usage guidance. Pure
/// USAGE guidance: the hook only fires while this worker is present, so it
/// carries no "look for it / install it" discovery text.
const CODE_RUNNER_GUIDANCE: &str = "sandbox-code-runner runs Node.js and Python in isolated microVMs (iii-sandbox). `sandbox-code-runner::run` with lang \"node\" or \"python\" is ONE-SHOT by default: it boots a fresh VM, runs the code, returns the result, and destroys the VM — nothing persists, no files, no installed packages, and the response carries no runtime_id (there is nothing left to address). Pass keep: true to leave the VM running instead: the response's runtime_id then addresses it — treat it as a secret — and is the capability `sandbox-code-runner::teardown` needs. Pass that runtime_id back on a later run to keep working in the same VM (filesystem persists between runs in one runtime; variables do not) — that runtime is never auto-stopped, and a reuse can fail with sandbox-code-runner::expired if it was idle-reaped; if it does, just run again the same way (fresh keep: true, or a fresh one-shot) rather than reusing the dead id. Every VM boots with outbound network, so npm/pip installs work on any path. Run code and registered handlers get a global `iii` — the REAL iii-sdk client, lazily connected to the engine on first use: `await iii.trigger({ function_id: 'worker::fn', payload })` in Node, `iii.trigger({'function_id': 'worker::fn', 'payload': ...})` (synchronous) in Python, with the full SDK surface behind it (registerFunction, registerTrigger, and the rest). Two sharp edges: functions registered with iii.registerFunction are EPHEMERAL — they die when the run or handler process exits, so persist through sandbox-code-runner::register_function (callable via iii.trigger); and a handler that triggers a function registered on the very runtime it executes in waits on that runtime's one-exec-at-a-time slot and can only time out — call across runtimes or workers instead. `sandbox-code-runner::register_function` needs no runtime_id at all: pass function_id, source (must define handler(payload) in lang), description, and lang — sandbox-code-runner keeps one persistent runtime per namespace (the segment of function_id before `::`) and language automatically, creating it on the first registration and reusing it for later ones in the same namespace and lang. Call `sandbox-code-runner::teardown` with EITHER runtime_id (a kept run's runtime) or namespace (e.g. \"app\" for ids like app::greet) — never both, never neither — to unregister its functions and stop its microVM(s). Idle runtimes are reaped after the configured TTL, but a reaped runtime's functions are NOT unregistered at that moment: the next call into it fails with sandbox-code-runner::expired, and only then are its functions unregistered. Don't assume a function id is free to reuse just because the TTL has passed.";

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
            system_prompt: Some(format!("{base}\n\n{CODE_RUNNER_GUIDANCE}")),
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
        assert!(
            sp.contains("sandbox-code-runner::run"),
            "guidance content present"
        );
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
        // Each needle is a fact an agent gets wrong without the guidance:
        // the three function ids, that run is one-shot unless kept, that
        // runtime_id is the thing to reuse, the handler signature
        // register_function expects, that it needs no runtime_id, and
        // that a run reuse can come back sandbox-code-runner::expired.
        // (Every VM being networked is pinned separately below, in
        // guidance_states_the_iii_global_rules_plainly's "outbound
        // network" needle — no need to duplicate it here.)
        for needle in [
            "sandbox-code-runner::run",
            "sandbox-code-runner::register_function",
            "sandbox-code-runner::teardown",
            "runtime_id",
            "keep: true",
            "handler(payload)",
            "sandbox-code-runner::expired",
            "namespace",
            "iii.trigger",
            "iii.registerFunction",
            "iii-sdk",
        ] {
            assert!(
                CODE_RUNNER_GUIDANCE.contains(needle),
                "guidance is missing: {needle}"
            );
        }
    }

    /// The core behavior change this guidance must state plainly, not hedge:
    /// run is one-shot by default and nothing persists unless `keep: true`,
    /// and `register_function` needs no `runtime_id` at all. A wrong or
    /// vague claim here becomes an agent's confident wrong belief.
    #[test]
    fn guidance_states_one_shot_eval_and_runtime_id_free_register_plainly() {
        assert!(
            CODE_RUNNER_GUIDANCE.contains("ONE-SHOT by default"),
            "guidance must state plainly that run defaults to one-shot"
        );
        assert!(
            CODE_RUNNER_GUIDANCE.contains("nothing persists, no files, no installed packages"),
            "guidance must state plainly that a one-shot run leaves nothing behind"
        );
        assert!(
            CODE_RUNNER_GUIDANCE.contains("needs no runtime_id at all"),
            "guidance must state plainly that register_function needs no runtime_id"
        );
        assert!(
            !CODE_RUNNER_GUIDANCE.contains("session"),
            "session binding was removed; the guidance must not mention it"
        );
    }

    /// The iii-global claims an agent gets wrong without them: it is the
    /// real SDK client, SDK-made registrations die with the guest process,
    /// same-runtime self-calls stall out, and every VM is networked.
    #[test]
    fn guidance_states_the_iii_global_rules_plainly() {
        for needle in [
            "REAL iii-sdk client",
            "EPHEMERAL",
            "one-exec-at-a-time",
            "outbound network",
        ] {
            assert!(
                CODE_RUNNER_GUIDANCE.contains(needle),
                "guidance is missing: {needle}"
            );
        }
    }
}
