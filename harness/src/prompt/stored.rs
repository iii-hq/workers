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

/// The effective default identity: the stored `default` prompt body when
/// present and non-empty, else the embedded [`super::DEFAULT`].
pub async fn effective_default(iii: &IIIClient) -> String {
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
pub fn choose_identity(stored_body: Option<String>) -> String {
    match stored_body {
        Some(body) if !body.trim().is_empty() => body,
        _ => super::DEFAULT.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_blank_stored_body_overrides_the_embedded_default() {
        assert_eq!(
            choose_identity(Some("You are CUSTOM.".into())),
            "You are CUSTOM."
        );
    }

    #[test]
    fn absent_or_blank_stored_body_falls_back_to_embedded() {
        assert_eq!(choose_identity(None), crate::prompt::DEFAULT);
        assert_eq!(choose_identity(Some(String::new())), crate::prompt::DEFAULT);
        assert_eq!(
            choose_identity(Some(" \n\t ".into())),
            crate::prompt::DEFAULT
        );
    }
}
