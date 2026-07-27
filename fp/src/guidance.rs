//! `fp::inject-guidance` — a `pre_generate` hook that contributes the
//! `fp::pipe` / transform usage guidance to the agent's system prompt,
//! ONLY while this worker is connected. The hook is bound at worker startup;
//! binding order does not matter: the engine parks a binding whose trigger
//! type is not registered yet as a pending intent and activates it when the
//! type appears ("recoverable triggers", engine/src/trigger.rs — this also
//! covers a harness restart, which re-parks and re-activates the binding).
//! The guidance stays presence-gated: without the fp worker the hook
//! function is gone and the fail_open binding contributes nothing. Mirrors
//! web/src/functions/inject_guidance.rs, minus the pre-recoverable-triggers
//! registry-watching rebind dance web still carries.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::trigger::Trigger;
use iii_sdk::{IIIClient, RegisterFunction};

pub const GUIDANCE_HOOK_ID: &str = "fp::inject-guidance";
pub const GUIDANCE_HOOK_DESC: &str =
    "Internal pre_generate hook: appends fp::pipe usage guidance to the agent system \
     prompt. Bound to harness::hook::pre-generate at worker startup; not called directly.";

/// The harness-PROVIDED trigger type this worker binds to (NOT an engine
/// built-in).
const PRE_GENERATE_TRIGGER_TYPE: &str = "harness::hook::pre-generate";

/// The pipe/transform usage guidance appended to the system prompt — the
/// single canonical copy (previously a hand-maintained section in the harness
/// prompt variants). Pure USAGE guidance: the hook only fires when the fp
/// worker is present, so no "look for it / install it" discovery text.
const GUIDANCE: &str = r#"## Moving big values between functions

HARD RULE: bulk data does not pass through you. A value you will not personally reason about
— a fetched document, a big function result, a list to reshape — moves worker→worker via
`fp::pipe`, never through the chat. Re-typing a big result into another call's arguments
stalls the provider stream mid-call for minutes and has killed live turns, besides burning
the context window; fetching a whole document "to look at it" floods the context for good.

`fp::pipe` runs the move in ONE call: each step triggers a function and its result lands
in the next step's payload at `into` (default "/value"). Pure transforms run inline as steps —
`fp::{get, pick, omit, take, drop, map, filter, split, join, uniq, size, compact, nth,
getOr, flatten, sortBy, reverse, when}` with lodash
semantics — their input arrives at `value`, and what they thread onward is the transformed
value itself (the `{value}` wrapper appears only on direct calls, never between steps).
Transform args beyond `value` (no schema lookup needed): get{path} getOr{path,default}
pick/omit{paths} take/drop{n} nth{n, -1=last} map{path} filter{matches: partial object}
split/join{separator} sortBy{path} when{path?,op,to?} — the rest take only `value`.
Producer responses are OBJECTS (search: {content_matches,path_matches}, grep: {matches},
exec: {stdout,…}, fetch: {content,…}) — `fp::get` the array/string field out before
reshaping it.
Producer steps too: `state::get` in a pipe hands the stored value onward BARE — never follow
it with `fp::get { path: "/value" }`; state::get's response IS the stored value (the `{value}`
wrapper is the transforms' direct-call shape, never a producer's).
`fp::when { path?, op, to? }` is the guard step: a failing comparison STOPS the pipe
(`short_circuited: true`), so a trailing write (e.g. `state::set`) runs only when the
condition holds — the primitive for mechanical threshold checks.
The FIRST step has no previous value to receive:
start with a producing function (a fetch, a state::get) or seed a leading transform via its
payload's `value`. You receive per-step sizes and a preview — never the value itself:

    fp::pipe { through: [
      { function: "scrapling::fetch",
        payload:  { url: "https://…", format: "markdown", main_content_only: true } },
      { function: "fp::get",  payload: { path: "/content" } },
      { function: "fp::take", payload: { n: 20000 } },
      { function: "state::set",   payload: { scope: "research", key: "article" } }
    ]}

Before any call that returns or carries bulk data, ask: do I need to READ this, or just
move/reshape/store it? Just moving → pipe. Need a slice to reason over → end the pipe in a
transform and read the receipt's preview (`preview_chars` sizes it, default 400, cap 8000). A small
result you must read in full → a plain direct call (the rule is about bulk, not ceremony) —
but NEVER probe a bulk function directly "to see its result shape": read the response schema
via engine::functions::info, or probe through a pipe (a `fp::get` with a wrong `path`
fails cheaply, naming the keys that WERE available). Plan ONE pipe per outcome: literal
fields in the final step's payload plus `into` compose the stored shape in place — ending in
{ function: "state::set", payload: { scope, key, value: { url } }, into: "/value/content" }
stores { url, content } in one step; don't store, inspect, re-store.
`shell::*`/`coder::*` steps work in a pipe and run under the session's filesystem scope —
the pattern for bulk code results: `coder::search` → `fp::get {path:"/content_matches"}` →
`fp::map {path:"/path"}` → `fp::uniq` → `state::set`, one call, nothing through the chat.
Caveat: a path-access rejection inside a pipe just fails
the step — only a DIRECT call offers the access-grant prompt.
Trigger-control, session/approval, credential/config, LLM-routing, and
turn-control steps are excluded — call those directly."#;

/// The slice of the `pre_generate` hook envelope we read (lenient: ignores
/// every other field the harness sends). The harness nests the live
/// generation context under `generate` (see harness
/// `HookRunner::run_pre_generate`).
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

/// The harness applies `system_prompt` only when the key is present
/// (`HookRunner::run_pre_generate` — `if m.system_prompt.is_some()`), so
/// `None` serializes to an empty object: the safe no-op that preserves the
/// harness's assembled prompt.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct PreGenerateMutations {
    /// Full replacement system prompt (base + appended guidance). The harness
    /// overwrites, it does not merge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

/// Build the `pre_generate` mutations for a given base prompt. Pure, so it's
/// unit-testable.
///
/// Returns NO `system_prompt` when `base` is empty. A missing or renamed
/// `generate.system_prompt` field deserializes to `""` (schema drift), and a
/// fail-open hook must PRESERVE the harness's assembled prompt, never replace
/// it with the guidance alone. For a real, non-empty base we append the
/// guidance and return the FULL prompt.
fn mutations_for(base: &str) -> PreGenerateMutations {
    if base.is_empty() {
        PreGenerateMutations::default()
    } else {
        PreGenerateMutations {
            system_prompt: Some(format!("{base}\n\n{GUIDANCE}")),
        }
    }
}

/// `pre_generate` hook entrypoint: return a `system_prompt` mutation that
/// appends the pipe guidance to a non-empty base. Bound `fail_open`, so an
/// error here never blocks a turn.
async fn handle(event: PreGenerateEvent) -> Result<PreGenerateResponse, Error> {
    Ok(PreGenerateResponse {
        mutations: mutations_for(&event.generate.system_prompt),
    })
}

/// Best-effort trigger binding: a transient failure must not brick boot — it
/// surfaces as a `None` handle.
fn bind(iii: &IIIClient, trigger_type: &str, function_id: &str, config: Value) -> Option<Trigger> {
    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: trigger_type.to_string(),
        function_id: function_id.to_string(),
        config,
        metadata: None,
        namespace: iii.namespace(),
    }) {
        Ok(handle) => {
            tracing::info!(trigger_type, function_id, "trigger binding requested");
            Some(handle)
        }
        Err(e) => {
            tracing::warn!(trigger_type, function_id, error = %e, "trigger binding failed");
            None
        }
    }
}

/// Register the guidance hook function and bind it to the harness
/// pre-generate trigger type. One shot: if the harness is not up yet, the
/// engine parks the binding as a pending intent and activates it when the
/// type registers (recoverable triggers), so there is nothing to watch or
/// retry. `on_error: fail_open` is MANDATORY: pre_generate defaults
/// fail-CLOSED, which would abort generation if this hook ever errored/timed
/// out; a missing guidance section must never block a turn.
pub fn setup(iii: &Arc<IIIClient>) {
    iii.register_function(
        GUIDANCE_HOOK_ID,
        RegisterFunction::new_async(
            move |event: PreGenerateEvent| async move { handle(event).await },
        )
        .description(GUIDANCE_HOOK_DESC)
        .metadata(json!({ "internal": true })),
    );

    bind(
        iii,
        PRE_GENERATE_TRIGGER_TYPE,
        GUIDANCE_HOOK_ID,
        json!({ "on_error": "fail_open" }),
    );
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
        assert!(sp.contains("fp::pipe"), "guidance content is present");
    }

    #[test]
    fn empty_base_emits_no_system_prompt_mutation() {
        // A missing/malformed hook payload (system_prompt absent → "") must
        // PRESERVE the harness prompt: emit no system_prompt key so the
        // harness keeps its own, rather than replacing the whole prompt with
        // the guidance alone. The wire shape must stay `{"mutations": {}}` —
        // the harness applies system_prompt only when the key is present.
        let wire = serde_json::to_value(PreGenerateResponse {
            mutations: mutations_for(""),
        })
        .expect("response serializes");
        assert_eq!(wire, serde_json::json!({ "mutations": {} }));
    }

    /// Mirrors the registry publish gate (`collect_worker_interface.py`): the
    /// derived response schema must carry a schema-defining keyword, not the
    /// AnyValue schema.
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
    fn guidance_mandates_present() {
        // The HARD RULE and its teachable edges must survive edits — these
        // moved here from the static harness prompts.
        for needle in [
            "HARD RULE",
            "fp::pipe",
            "`into` (default \"/value\")",
            "fp::{get, pick, omit, take, drop, map, filter, split, join, uniq, size, compact, nth,",
            "getOr, flatten, sortBy, reverse, when}",
            "short_circuited",
            "wrapper appears only on direct calls",
            "hands the stored value onward BARE",
            "never a producer's",
            "The FIRST step has no previous value to receive",
            "preview_chars",
            "the rule is about bulk, not ceremony",
            "engine::functions::info",
            "ONE pipe per outcome",
            "into: \"/value/content\"",
            "`shell::*`/`coder::*` steps work in a pipe",
            "only a DIRECT call offers the access-grant prompt",
            "no schema lookup needed",
            "Producer responses are OBJECTS",
            "fp::get {path:\"/content_matches\"}",
        ] {
            assert!(GUIDANCE.contains(needle), "missing: {needle}");
        }
    }
}
