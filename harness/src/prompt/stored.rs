//! The stored default identity. A `default` entry in the directory's
//! system-prompt store (`<skills_folder>/system-prompts/default.md`, served
//! by `directory::system-prompts::get`) overrides the embedded identity
//! prompt for every NEW prompt composition; editing or deleting that file
//! hot-applies on the next send. Any store failure — directory not
//! installed, not running, entry absent, empty body — falls back to the
//! embedded prompt: the override can never block a send.
//!
//! The lookup must also stay ERROR-SPAN-CLEAN on the fallback paths: this
//! resolution runs on every send, so an expected miss must never stamp an
//! error span into the turn's trace (the integration floor rejects turns
//! with error spans, and production traces would carry one per send).
//! Hence the ladder below: fail-open presence probe, OK-shaped existence
//! check, and only then the direct get.

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use crate::trace_tags::run_hidden;

/// The reserved store entry name that overrides the embedded default.
pub const STORED_DEFAULT_PROMPT_NAME: &str = "default";
const GET_ID: &str = "directory::system-prompts::get";
const LIST_ID: &str = "directory::system-prompts::list";
const FUNCTIONS_LIST_ID: &str = "engine::functions::list";
const HIDDEN_FAMILY: &str = "harness prompt";
const TIMEOUT_MS: u64 = 5_000;

/// The resolved default identity plus WHERE it came from — the source is
/// carried from the lookup itself so callers never have to infer it by
/// comparing bodies (a stored entry whose body happens to equal the
/// embedded prompt is still a stored entry).
pub struct EffectiveDefault {
    pub identity: String,
    /// The identity is the stored `system-prompts/default` entry, not the
    /// embedded prompt.
    pub stored: bool,
}

/// The effective default identity: the stored `default` prompt body when
/// present and non-empty, else the embedded [`super::DEFAULT`].
pub async fn effective_default(iii: &IIIClient) -> EffectiveDefault {
    choose_identity(stored_default_body(iii).await)
}

/// The stored `default` body, resolved through a span-clean ladder:
///
/// 1. Presence — `engine::functions::list` is fail-open and answers OK
///    (possibly empty) even when the directory worker is absent; a direct
///    get there would `function_not_found` and error-stamp every send of a
///    directory-less deployment.
/// 2. Existence — `directory::system-prompts::list` answers OK whether or
///    not a `default` entry exists; the get's miss is a handler ERROR and
///    would error-stamp every send of every no-override deployment.
/// 3. Only then fetch the body.
async fn stored_default_body(iii: &IIIClient) -> Option<String> {
    let functions = trigger(iii, FUNCTIONS_LIST_ID, json!({ "prefix": GET_ID })).await?;
    let present = functions
        .get("functions")
        .and_then(Value::as_array)
        .is_some_and(|functions| !functions.is_empty());
    if !present {
        return None;
    }
    let prompts = trigger(iii, LIST_ID, json!({})).await?;
    let has_default = prompts
        .get("prompts")
        .and_then(Value::as_array)
        .is_some_and(|prompts| {
            prompts
                .iter()
                .any(|p| p.get("name").and_then(Value::as_str) == Some(STORED_DEFAULT_PROMPT_NAME))
        });
    if !has_default {
        return None;
    }
    trigger(iii, GET_ID, json!({ "name": STORED_DEFAULT_PROMPT_NAME }))
        .await?
        .get("body")
        .and_then(Value::as_str)
        .map(str::to_string)
}

async fn trigger(iii: &IIIClient, function_id: &str, payload: Value) -> Option<Value> {
    run_hidden(
        HIDDEN_FAMILY,
        iii.trigger(TriggerRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(TIMEOUT_MS),
        }),
    )
    .await
    .ok()
}

/// Pure selection: a non-blank stored body wins; anything else is embedded.
pub fn choose_identity(stored_body: Option<String>) -> EffectiveDefault {
    match stored_body {
        Some(body) if !body.trim().is_empty() => EffectiveDefault {
            identity: body,
            stored: true,
        },
        _ => EffectiveDefault {
            identity: super::DEFAULT.to_string(),
            stored: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_blank_stored_body_overrides_the_embedded_default() {
        let chosen = choose_identity(Some("You are CUSTOM.".into()));
        assert_eq!(chosen.identity, "You are CUSTOM.");
        assert!(chosen.stored);
    }

    /// The source flag comes from the lookup, never from comparing bodies:
    /// a stored entry that duplicates the embedded text is still stored.
    #[test]
    fn stored_body_equal_to_the_embedded_prompt_still_reports_stored() {
        let chosen = choose_identity(Some(crate::prompt::DEFAULT.to_string()));
        assert_eq!(chosen.identity, crate::prompt::DEFAULT);
        assert!(chosen.stored);
    }

    #[test]
    fn absent_or_blank_stored_body_falls_back_to_embedded() {
        for stored in [None, Some(String::new()), Some(" \n\t ".into())] {
            let chosen = choose_identity(stored);
            assert_eq!(chosen.identity, crate::prompt::DEFAULT);
            assert!(!chosen.stored);
        }
    }
}
