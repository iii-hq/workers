//! `web::inject-guidance` — a `pre_generate` hook that contributes the `web::fetch`
//! usage guidance to the agent's system prompt, ONLY while this worker is connected.
//! The hook is bound at worker startup and the binding is dropped when the worker goes
//! away, so the guidance is presence-gated for free: a deployment without the web
//! worker never pays for it, and it is no longer hand-duplicated (and drifting) across
//! the static harness prompt variants.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

pub const GUIDANCE_HOOK_ID: &str = "web::inject-guidance";
pub const GUIDANCE_HOOK_DESC: &str =
    "Internal pre_generate hook: appends web::fetch usage guidance to the agent system \
     prompt. Bound to harness::hook::pre-generate at worker startup; not called directly.";

/// The `web::fetch` usage guidance appended to the system prompt — the single canonical
/// copy (previously hand-duplicated, and drifting, across the harness prompt variants).
/// Pure USAGE guidance: the hook only fires when the web worker is present, so the old
/// "look for it / install it" discovery text is unnecessary.
const WEB_GUIDANCE: &str = "For any HTTP(S) request — fetching a URL, calling a JSON/REST API, or downloading a file — ALWAYS use `web::fetch`, never `shell::exec` with `curl` or `wget`. It returns a parsed `{ ok, status, headers, body }` envelope, enforces size/timeout caps, and applies server-side SSRF protection a shell `curl` cannot. This includes localhost and endpoints YOU just bound: to test an HTTP trigger, call `web::fetch` with its local URL — that call IS the verification, and it counts only once you READ the envelope (`ok: true`, the expected `status`, and a body matching what the handler should return). There is no quick-local-test exception for `curl`. To READ a web page or docs, pass `format: \"markdown\"` — it converts HTML to compact Markdown instead of raw HTML that floods your context. `ok: true` means the request completed, not that HTTP succeeded — branch on `status` for 4xx/5xx and on `error` (one of invalid_payload, invalid_url, blocked_host, timeout, too_many_redirects, transport_error), never on message text. Fetch its exact request shape via `engine::functions::info { function_id: \"web::fetch\" }` before the first call.";

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

/// Append the web guidance to the base prompt. Pure, so it's unit-testable. The harness
/// OVERWRITES `system_prompt` with what we return (it does not merge), so we must return
/// the FULL prompt (base + guidance), not just the addition.
fn enrich(base: &str) -> String {
    if base.is_empty() {
        WEB_GUIDANCE.to_string()
    } else {
        format!("{base}\n\n{WEB_GUIDANCE}")
    }
}

/// `pre_generate` hook entrypoint: return a `system_prompt` mutation that appends the
/// web guidance. Bound `fail_open`, so an error here never blocks a turn.
pub async fn handle(event: PreGenerateEvent) -> Result<Value, iii_sdk::errors::Error> {
    Ok(json!({ "mutations": { "system_prompt": enrich(&event.generate.system_prompt) } }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrich_appends_guidance_after_base() {
        let out = enrich("BASE PROMPT");
        assert!(
            out.starts_with("BASE PROMPT\n\n"),
            "the base prompt must be preserved, guidance appended after it"
        );
        assert!(out.contains("web::fetch"), "guidance content is present");
        assert!(
            out.contains("format: \"markdown\""),
            "page-reading guidance is present"
        );
    }

    #[test]
    fn enrich_handles_empty_base() {
        // A missing/empty base must not produce a leading blank block.
        assert_eq!(enrich(""), WEB_GUIDANCE);
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
