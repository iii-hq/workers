//! `directory::inject-guidance` — a `pre_generate` hook that points the
//! agent's system prompt at the skills index, ONLY while this worker is
//! connected. The hook is bound at worker startup; the engine parks a
//! binding whose trigger type is not registered yet as a pending intent
//! and activates it when the type appears (recoverable triggers), which
//! also covers a harness restart. Presence-gated: without iii-directory
//! the hook function is gone and the fail_open binding contributes
//! nothing. Mirrors fp/src/guidance.rs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};

const GUIDANCE_HOOK_ID: &str = "directory::inject-guidance";
const GUIDANCE_HOOK_DESC: &str =
    "Internal pre_generate hook: appends a pointer to the worker skills index to the agent \
     system prompt. Bound to harness::hook::pre-generate at worker startup; not called directly.";

/// The harness-PROVIDED trigger type this worker binds to (NOT an engine
/// built-in).
const PRE_GENERATE_TRIGGER_TYPE: &str = "harness::hook::pre-generate";

/// The pointer appended to the system prompt. Deliberately a pointer, not
/// content: the index body stays out of every turn's context and is paid
/// for only when the agent actually calls the function.
const GUIDANCE: &str = "## Worker knowledge index\n\n\
A skills index covering every connected worker exists. Call directory::skills::index for a \
one-paragraph map of what each worker does, then directory::skills::get { \"id\": \"<worker>\" } \
for the full guide before first use of an unfamiliar worker.";

/// The slice of the `pre_generate` hook envelope we read (lenient:
/// ignores every other field the harness sends).
#[derive(Debug, Default, Deserialize, JsonSchema)]
struct PreGenerateEvent {
    #[serde(default)]
    generate: GenerateContext,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct GenerateContext {
    /// The system prompt assembled so far (base + any prior hook's mutation).
    #[serde(default)]
    system_prompt: String,
}

/// Hook envelope returned to the harness: the mutations to apply.
#[derive(Debug, Serialize, JsonSchema)]
struct PreGenerateResponse {
    mutations: PreGenerateMutations,
}

/// The harness applies `system_prompt` only when the key is present, so
/// `None` serializes to an empty object: the safe no-op that preserves
/// the harness's assembled prompt.
#[derive(Debug, Default, Serialize, JsonSchema)]
struct PreGenerateMutations {
    /// Full replacement system prompt (base + appended pointer). The
    /// harness overwrites, it does not merge.
    #[serde(skip_serializing_if = "Option::is_none")]
    system_prompt: Option<String>,
}

/// Build the `pre_generate` mutations for a given base prompt. Pure, so
/// it's unit-testable.
///
/// Returns NO `system_prompt` when `base` is empty (a missing or renamed
/// `generate.system_prompt` field deserializes to `""`, and a fail-open
/// hook must preserve the harness's assembled prompt, never replace it
/// with the pointer alone) or when the base already mentions the index
/// function (avoids duplicating a prompt variant that covers it).
fn mutations_for(base: &str) -> PreGenerateMutations {
    if base.is_empty() || base.contains("directory::skills::index") {
        PreGenerateMutations::default()
    } else {
        PreGenerateMutations {
            system_prompt: Some(format!("{base}\n\n{GUIDANCE}")),
        }
    }
}

async fn handle(event: PreGenerateEvent) -> Result<PreGenerateResponse, Error> {
    Ok(PreGenerateResponse {
        mutations: mutations_for(&event.generate.system_prompt),
    })
}

/// Register the guidance hook function and bind it to the harness
/// pre-generate trigger type. One shot: if the harness is not up yet,
/// the engine parks the binding and activates it when the type registers.
/// `on_error: fail_open` is MANDATORY: pre_generate defaults fail-CLOSED,
/// which would abort generation if this hook ever errored or timed out; a
/// missing pointer must never block a turn.
pub fn setup(iii: &IIIClient) {
    iii.register_function(
        GUIDANCE_HOOK_ID,
        RegisterFunction::new_async(
            move |event: PreGenerateEvent| async move { handle(event).await },
        )
        .description(GUIDANCE_HOOK_DESC)
        .metadata(json!({ "internal": true })),
    );

    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: PRE_GENERATE_TRIGGER_TYPE.to_string(),
        function_id: GUIDANCE_HOOK_ID.to_string(),
        config: json!({ "on_error": "fail_open" }),
        metadata: None,
    }) {
        Ok(_) => tracing::info!(
            trigger_type = PRE_GENERATE_TRIGGER_TYPE,
            function_id = GUIDANCE_HOOK_ID,
            "guidance hook binding requested"
        ),
        Err(e) => tracing::warn!(
            trigger_type = PRE_GENERATE_TRIGGER_TYPE,
            function_id = GUIDANCE_HOOK_ID,
            error = %e,
            "guidance hook binding failed"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_base_is_a_no_op() {
        let m = mutations_for("");
        assert!(m.system_prompt.is_none(), "empty base must not be replaced");
    }

    #[test]
    fn base_mentioning_index_is_a_no_op() {
        let m = mutations_for("Use directory::skills::index for discovery.");
        assert!(
            m.system_prompt.is_none(),
            "a base that already covers the index must not be duplicated"
        );
    }

    #[test]
    fn pointer_is_appended_to_base() {
        let m = mutations_for("You are an agent.");
        let prompt = m.system_prompt.expect("pointer appended");
        assert!(prompt.starts_with("You are an agent."));
        assert!(prompt.contains("Worker knowledge index"));
    }

    #[test]
    fn no_op_serializes_to_empty_mutations() {
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
}
