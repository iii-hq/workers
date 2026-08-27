//! The stored default identity. A `default` entry in the directory's
//! system-prompt store (`<skills_folder>/system-prompts/default.md`, served
//! by `directory::system-prompts::get`) overrides the embedded identity
//! prompt for every NEW prompt composition; editing or deleting that file
//! hot-applies on the next send. Any store failure — directory not
//! installed, not running, entry absent, empty body — falls back to the
//! embedded prompt: the override can never block a send.

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::json;

use crate::trace_tags::run_hidden;

/// The reserved store entry name that overrides the embedded default.
pub const STORED_DEFAULT_PROMPT_NAME: &str = "default";
const GET_ID: &str = "directory::system-prompts::get";
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
    let stored = run_hidden(
        HIDDEN_FAMILY,
        iii.trigger(TriggerRequest {
            function_id: GET_ID.into(),
            payload: json!({ "name": STORED_DEFAULT_PROMPT_NAME }),
            action: None,
            timeout_ms: Some(TIMEOUT_MS),
        }),
    )
    .await
    .ok()
    .and_then(|value| {
        value
            .get("body")
            .and_then(|body| body.as_str())
            .map(str::to_string)
    });
    choose_identity(stored)
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
