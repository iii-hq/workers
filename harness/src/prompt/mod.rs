//! System-prompt assembly: the single embedded harness-owned identity prompt.
//! The identity is deliberately minimal and engine-grounded — the basic
//! engine functions and the discovery loop (list, info, call); everything
//! else the agent discovers from the live engine.

mod stored;
mod variants;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use stored::{effective_default, EffectiveDefault, STORED_DEFAULT_PROMPT_NAME};
pub use variants::DEFAULT;

/// How a caller-supplied system prompt combines with the built-in identity prompt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SystemPromptStrategy {
    /// Caller prompt replaces the built-in prompt verbatim.
    Override,
    /// Caller prompt is appended to the built-in identity prompt.
    #[default]
    Enrich,
    /// No system prompt is sent to the model.
    Disabled,
}

/// Resolve the system prompt for a turn. With `Override`, a non-empty caller
/// prompt wins verbatim; with `Enrich`, it is appended to the built-in prompt.
/// An absent or empty caller prompt yields the built-in prompt under
/// `Override` and `Enrich`. `Disabled` always yields no system prompt.
pub fn resolve_system_prompt(
    override_prompt: Option<String>,
    strategy: SystemPromptStrategy,
    identity: &str,
) -> Option<String> {
    let custom = override_prompt.as_deref().filter(|s| !s.is_empty());
    match (strategy, custom) {
        (SystemPromptStrategy::Disabled, _) => None,
        (SystemPromptStrategy::Override, Some(s)) => Some(s.to_string()),
        (SystemPromptStrategy::Enrich, Some(s)) => Some(format!("{identity}\n\n{s}")),
        (_, None) => Some(identity.to_string()),
    }
}

#[cfg(test)]
mod tests;
