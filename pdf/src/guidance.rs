//! `pdf::inject-guidance` — a `pre_generate` hook that tells the agent this
//! worker exists, and only while it is connected.
//!
//! Without it an agent that meets a PDF path does the thing it has always done:
//! reads the file as text and gets binary noise, or gives up. The guidance is
//! short on purpose. The harness reserves a fixed token allowance whenever a
//! pre-generate hook is bound, and a long injection eats into the turn.
//!
//! `on_error: fail_open` is mandatory. Pre-generate hooks default to fail
//! CLOSED, so a hook that errored or timed out would abort generation. A
//! missing paragraph of advice must never kill a turn.
//!
//! Mirrors `fp/src/guidance.rs`, which is the reference for this seam.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::trigger::Trigger;
use iii_sdk::{IIIClient, RegisterFunction};

pub const GUIDANCE_HOOK_ID: &str = "pdf::inject-guidance";
pub const GUIDANCE_HOOK_DESC: &str =
    "Internal pre_generate hook: appends pdf::* usage guidance to the agent system prompt. \
     Bound to harness::hook::pre-generate at worker startup; not called directly.";

/// The harness-PROVIDED trigger type this worker binds to (not an engine
/// built-in).
const PRE_GENERATE_TRIGGER_TYPE: &str = "harness::hook::pre-generate";

/// Pure usage guidance: the hook only fires while this worker is connected, so
/// there is no discovery or installation text.
const GUIDANCE: &str = r#"## Reading PDFs

A PDF is not text. Reading one with a file-reading function returns binary noise and burns
the context on it. Every PDF goes through `pdf::*`, which parses the document locally in
milliseconds, needs no key, and sends nothing anywhere.

`pdf::classify` FIRST, always. It samples the document in about twenty milliseconds and tells
you whether the pages hold real text (`text_based`), are photographs of pages (`scanned` /
`image_based`), or are a mix. Extraction on a scan returns nothing, so classifying first is
what stops you reporting an empty document as an empty document. Read `pages_needing_ocr` and
`ocr_reasons`: `scanned` and `no_text` mean those pages need a vision model, `vector_text`
means the characters are drawn as shapes, and `suspected_garbled_text` means a text layer
that decodes to nonsense and must not be trusted no matter how confident the type looks.

Then pick by what you need:
`pdf::to-markdown` to READ it — keeps headings, lists, links and tables.
`pdf::extract-text` to search or embed it — cheaper, no structure.
`pdf::extract-items` for WHERE text sits — boxes, fonts, sizes, styling.
`pdf::extract-regions` when you already know the box you care about.

Responses are capped and say so. `truncated: true` with a `total_chars` far above `chars`
means you are holding a fragment: do not answer from it. Narrow with `pages` (1-indexed)
rather than raising the cap — a targeted page beats a truncated document. `max_chars: 0`
lifts the cap entirely and belongs in an `fp::pipe` moving the document to storage, never in
a call whose result lands in this conversation.

Page numbers on this surface are 1-indexed everywhere, in requests and responses.
Coordinates differ by function and each response states which it used: `pdf::extract-items`
reports PDF points from the BOTTOM left, `pdf::extract-regions` takes boxes in PDF points
from the TOP left."#;

/// The slice of the `pre_generate` hook envelope this worker reads. Lenient: it
/// ignores every other field the harness sends.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PreGenerateEvent {
    #[serde(default)]
    pub generate: GenerateContext,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GenerateContext {
    /// The system prompt assembled so far (base plus any prior hook's mutation).
    #[serde(default)]
    pub system_prompt: String,
}

/// Hook envelope returned to the harness.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PreGenerateResponse {
    pub mutations: PreGenerateMutations,
}

/// The harness applies `system_prompt` only when the key is present, so `None`
/// serializes to an empty object: the safe no-op that preserves the harness's
/// assembled prompt.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct PreGenerateMutations {
    /// Full replacement system prompt (base plus appended guidance). The
    /// harness overwrites, it does not merge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

/// Build the mutations for a given base prompt. Pure, so it is unit-testable.
///
/// Returns NO `system_prompt` when `base` is empty. A missing or renamed
/// `generate.system_prompt` deserializes to `""`, and a fail-open hook must
/// PRESERVE the harness's prompt rather than replace the whole thing with the
/// guidance alone.
fn mutations_for(base: &str) -> PreGenerateMutations {
    if base.is_empty() {
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

/// Best-effort binding: a transient failure must not brick boot.
fn bind(iii: &IIIClient, trigger_type: &str, function_id: &str, config: Value) -> Option<Trigger> {
    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: trigger_type.to_string(),
        function_id: function_id.to_string(),
        config,
        metadata: None,
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

/// Whether to inject at all.
///
/// The hook's whole job is to advertise `pdf::*` in the agent's system prompt.
/// That is right for production and wrong for an evaluation of whether an agent
/// DISCOVERS the worker unaided, since an advertised worker cannot be
/// discovered. Off by absence: unset means inject.
fn guidance_enabled() -> bool {
    guidance_enabled_for(std::env::var("PDF_INJECT_GUIDANCE").ok().as_deref())
}

fn guidance_enabled_for(value: Option<&str>) -> bool {
    !matches!(value, Some("0" | "false" | "off"))
}

pub fn setup(iii: &Arc<IIIClient>) {
    if !guidance_enabled() {
        tracing::info!("PDF_INJECT_GUIDANCE disabled; pdf::* stays out of the agent system prompt");
        return;
    }
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
        assert!(sp.contains("pdf::classify"), "guidance content is present");
    }

    /// A missing or malformed hook payload must PRESERVE the harness prompt.
    /// The wire shape has to stay `{"mutations": {}}`, because the harness
    /// applies `system_prompt` only when the key is present.
    #[test]
    fn empty_base_emits_no_system_prompt_mutation() {
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
    fn guidance_can_be_switched_off_for_discovery_evals() {
        assert!(guidance_enabled_for(None));
        assert!(guidance_enabled_for(Some("1")));
        for off in ["0", "false", "off"] {
            assert!(
                !guidance_enabled_for(Some(off)),
                "{off} must disable injection"
            );
        }
    }

    /// The harness reserves a fixed token allowance for pre-generate hooks, and
    /// an injection that outgrows it costs the turn steps it needed. Keep the
    /// guidance to advice an agent acts on.
    #[test]
    fn guidance_stays_short() {
        assert!(
            GUIDANCE.len() < 2600,
            "guidance is {} bytes; trim it rather than eating the turn's token allowance",
            GUIDANCE.len()
        );
    }

    /// The load-bearing instructions must survive edits.
    #[test]
    fn guidance_mandates_present() {
        for needle in [
            "pdf::classify` FIRST",
            "A PDF is not text",
            "suspected_garbled_text",
            "pages_needing_ocr",
            "truncated: true",
            "max_chars: 0",
            "1-indexed",
            "BOTTOM left",
            "TOP left",
        ] {
            assert!(GUIDANCE.contains(needle), "missing: {needle}");
        }
    }

    /// Every function in the catalog should be reachable from the guidance, or
    /// an agent has no reason to ever call it.
    #[test]
    fn guidance_names_every_function() {
        for spec in crate::functions::catalog() {
            assert!(
                GUIDANCE.contains(spec.function_id),
                "guidance never mentions {}",
                spec.function_id
            );
        }
    }
}
