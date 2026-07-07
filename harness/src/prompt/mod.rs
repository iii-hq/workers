//! System-prompt assembly. The composed prompt is, top to bottom (each part
//! optional except the identity):
//!
//! 1. **Mode paragraph** (plan/ask/agent).
//! 2. **Identity** — the provider-served prompt (fetched from the llm-router
//!    at turn creation — providers declare it, operators may override or
//!    disable it in the llm-router config), with the embedded default prompt
//!    as fallback. Engine-grounded: the agent discovers capabilities from the
//!    live engine and installs registry workers when nothing fits.
//! 3. **Worker sections** — one per running worker that declares
//!    `agent_instructions` metadata on its configuration entry (or that the
//!    operator wrote per-worker text for), sorted by worker id. Operator text
//!    follows the worker's own text inside the section
//!    (see [`crate::instructions`]).
//! 4. **Global operator instructions** — the `instructions` entry's `global`.
//! 5. **Caller `system_prompt`** (per-send, `enrich`) — appended last by
//!    [`resolve_system_prompt`]; the most specific text gets the last word.
//!
//! Later parts refine earlier ones. `Override` skips ALL of it: the caller
//! prompt is served verbatim.

mod mode;
mod variants;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use mode::{paragraph, Mode};
pub use variants::DEFAULT;

/// One running worker's prompt section: its declared `agent_instructions`
/// text and/or the operator's per-worker text (at least one is set).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerSection {
    pub worker: String,
    pub declared: Option<String>,
    pub user: Option<String>,
}

fn render_section(section: &WorkerSection) -> String {
    let mut out = format!("# Notes from the `{}` worker", section.worker);
    for text in [section.declared.as_deref(), section.user.as_deref()]
        .into_iter()
        .flatten()
    {
        out.push_str("\n\n");
        out.push_str(text.trim());
    }
    out
}

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
    /// Running-worker sections, pre-gated and sorted (see
    /// [`crate::instructions::live_sections`]).
    pub sections: &'a [WorkerSection],
    /// Global operator instructions (the `instructions` entry's `global`).
    pub user_global: Option<&'a str>,
}

/// Build the built-in prompt: mode paragraph, identity, worker sections, and
/// global operator instructions, in that order (module doc has the contract).
pub fn build_system_prompt(opts: SystemPromptOpts<'_>) -> String {
    let identity = opts.identity.unwrap_or(variants::DEFAULT);
    let mut parts: Vec<String> = Vec::new();
    if let Some(mode) = opts.mode {
        parts.push(mode::paragraph(mode).to_string());
    }
    parts.push(identity.to_string());
    parts.extend(opts.sections.iter().map(render_section));
    if let Some(global) = opts.user_global.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(format!("# Operator instructions\n\n{global}"));
    }
    parts.join("\n\n")
}

/// Resolve the system prompt for a turn. With `Override`, a non-empty caller
/// prompt wins verbatim; with `Enrich`, it is appended to the built-in prompt.
/// An absent or empty caller prompt yields the built-in prompt under either
/// strategy.
pub fn resolve_system_prompt(
    override_prompt: Option<String>,
    strategy: SystemPromptStrategy,
    opts: SystemPromptOpts<'_>,
) -> Option<String> {
    let built_in = || build_system_prompt(opts);
    let custom = override_prompt.as_deref().filter(|s| !s.is_empty());
    match (strategy, custom) {
        (SystemPromptStrategy::Override, Some(s)) => Some(s.to_string()),
        (SystemPromptStrategy::Enrich, Some(s)) => Some(format!("{}\n\n{}", built_in(), s)),
        (_, None) => Some(built_in()),
    }
}

#[cfg(test)]
mod tests;
