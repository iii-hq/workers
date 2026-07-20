//! System-prompt assembly: a provider-served identity prompt (fetched from the
//! llm-router at turn creation — providers declare it, operators may override
//! or disable it in the llm-router config) with the embedded default prompt as
//! fallback, plus optional mode paragraphs. The default is engine-grounded —
//! the agent discovers capabilities from the live engine, installs registry
//! workers when nothing fits, routes code-file work through `coder::*`, and
//! fetches the iii.dev SDK reference before authoring workers.

mod mode;
mod variants;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use mode::{paragraph, Mode};
pub use variants::{DEFAULT, SUBAGENT};

/// How a caller-supplied system prompt combines with the built-in identity prompt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SystemPromptStrategy {
    /// Caller prompt replaces the built-in prompt verbatim.
    Override,
    /// Caller prompt is appended to the built-in identity prompt.
    #[default]
    Enrich,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPromptOpts<'a> {
    pub mode: Option<Mode>,
    /// Router-served per-provider identity prompt; the embedded default when absent.
    pub identity: Option<&'a str>,
}

/// Build the canonical identity prompt, optionally prefixed with a mode
/// paragraph.
pub fn build_system_prompt(opts: SystemPromptOpts<'_>) -> String {
    let identity = opts.identity.unwrap_or(variants::DEFAULT);
    match opts.mode {
        Some(mode) => format!("{}\n\n{}", mode::paragraph(mode), identity),
        None => identity.to_string(),
    }
}

/// Resolve the system prompt for a turn. With `Override`, a non-empty caller
/// prompt wins verbatim; with `Enrich`, it is appended to the built-in prompt.
/// An absent or empty caller prompt yields the built-in prompt under either
/// strategy.
pub fn resolve_system_prompt(
    override_prompt: Option<String>,
    strategy: SystemPromptStrategy,
    mode: Option<Mode>,
    identity: Option<&str>,
) -> Option<String> {
    let built_in = || build_system_prompt(SystemPromptOpts { mode, identity });
    let custom = override_prompt.as_deref().filter(|s| !s.is_empty());
    match (strategy, custom) {
        (SystemPromptStrategy::Override, Some(s)) => Some(s.to_string()),
        (SystemPromptStrategy::Enrich, Some(s)) => Some(format!("{}\n\n{}", built_in(), s)),
        (_, None) => Some(built_in()),
    }
}

#[cfg(test)]
mod tests;
