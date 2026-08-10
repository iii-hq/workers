//! `web::inject-guidance` — a `pre_generate` hook that contributes the `web::fetch`
//! usage guidance to the agent's system prompt, ONLY while this worker is connected.
//! The hook is bound at worker startup and the binding is dropped when the worker goes
//! away, so the guidance is presence-gated for free: a deployment without the web
//! worker never pays for it, and it is no longer hand-duplicated (and drifting) across
//! the static harness prompt variants.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const GUIDANCE_HOOK_ID: &str = "web::inject-guidance";
pub const GUIDANCE_HOOK_DESC: &str =
    "Internal pre_generate hook: appends web::fetch usage guidance to the agent system \
     prompt. Bound to harness::hook::pre-generate at worker startup; not called directly.";

/// The `web::fetch` usage guidance appended to the system prompt — the single canonical
/// copy (previously hand-duplicated, and drifting, across the harness prompt variants).
/// Pure USAGE guidance: the hook only fires when the web worker is present, so the old
/// "look for it / install it" discovery text is unnecessary.
pub(crate) const WEB_GUIDANCE: &str = "For any HTTP(S) request — fetching a URL, calling a JSON/REST API, or downloading a file — ALWAYS use `web::fetch`, never `shell::exec` with `curl` or `wget`. It returns a parsed `{ ok, status, headers, body }` envelope, enforces size/timeout caps, and applies server-side SSRF protection a shell `curl` cannot. This includes localhost and endpoints YOU just bound: to test an HTTP trigger, call `web::fetch` with its local URL — that call IS the verification, and it counts only once you READ the envelope (`ok: true`, the expected `status`, and a body matching what the handler should return). There is no quick-local-test exception for `curl`. To READ a web page or docs, pass `format: \"markdown\"` — it converts HTML to compact Markdown instead of raw HTML that floods your context. `ok: true` means the request completed, not that HTTP succeeded — branch on `status` for 4xx/5xx and on `error` (one of invalid_payload, invalid_url, blocked_host, timeout, too_many_redirects, transport_error), never on message text. Fetch its exact request shape via `engine::functions::info { function_id: \"web::fetch\" }` before its first call in this session.";

/// The slice of the `pre_generate` hook envelope we read (lenient: ignores every other
/// field the harness sends). The harness nests the live generation context under
/// `generate` (see harness `HookRunner::run_pre_generate`).
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

/// Hook envelope returned to the harness: the mutations to apply to the generation.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PreGenerateResponse {
    pub mutations: PreGenerateMutations,
}

/// The harness applies `system_prompt` only when the key is present
/// (`HookRunner::run_pre_generate` — `if m.system_prompt.is_some()`), so `None`
/// serializes to an empty object: the safe no-op that preserves the harness's
/// assembled prompt.
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
/// fail-open hook must PRESERVE the harness's assembled prompt, never replace it with
/// the guidance alone. For a real, non-empty base we append the guidance and return
/// the FULL prompt.
fn mutations_for(base: &str) -> PreGenerateMutations {
    if base.is_empty() {
        PreGenerateMutations::default()
    } else {
        PreGenerateMutations {
            system_prompt: Some(format!("{base}\n\n{WEB_GUIDANCE}")),
        }
    }
}

/// `pre_generate` hook entrypoint: return a `system_prompt` mutation that appends the
/// web guidance to a non-empty base. Bound `fail_open`, so an error here never blocks a
/// turn.
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
        assert!(sp.contains("web::fetch"), "guidance content is present");
        assert!(
            sp.contains("format: \"markdown\""),
            "page-reading guidance is present"
        );
    }

    #[test]
    fn empty_base_emits_no_system_prompt_mutation() {
        // A missing/malformed hook payload (system_prompt absent → "") must PRESERVE the
        // harness prompt: emit no system_prompt key so the harness keeps its own, rather
        // than replacing the whole prompt with the guidance alone. The wire shape must
        // stay `{"mutations": {}}` — the harness applies system_prompt only when the
        // key is present.
        let wire = serde_json::to_value(PreGenerateResponse {
            mutations: mutations_for(""),
        })
        .expect("response serializes");
        assert_eq!(wire, serde_json::json!({ "mutations": {} }));
    }

    /// Mirrors the registry publish gate (`collect_worker_interface.py`): the derived
    /// response schema must carry a schema-defining keyword, not the AnyValue schema
    /// (which blocked the web/v1.2.0 release).
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
    fn web_fetch_mandate_present() {
        // These assertions moved here from the harness prompt tests
        // (web_fetch_localhost_mandate): the mandate now lives in the injected
        // guidance, not the static prompt.
        for needle in [
            "includes localhost",
            "IS the verification",
            "web::fetch",
            "never `shell::exec` with",
            "`curl` or `wget`",
            "{ ok, status, headers, body }",
            "format: \"markdown\"",
        ] {
            assert!(WEB_GUIDANCE.contains(needle), "missing: {needle}");
        }
    }
}
