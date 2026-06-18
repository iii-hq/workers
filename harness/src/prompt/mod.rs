//! System-prompt assembly: per-model identity prompts (prompt/*) plus optional
//! mode paragraphs. Every variant is engine-grounded — the agent discovers
//! capabilities from the live engine, installs registry workers when nothing
//! fits, routes code-file work through `coder::*`, and fetches the iii.dev SDK
//! reference before authoring workers.

mod family;
mod mode;
mod variants;

pub use family::{identity_prompt, prompt_family, select_identity_prompt, PromptFamily};
pub use mode::{paragraph, Mode};

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPromptOpts<'a> {
    pub mode: Option<Mode>,
    pub provider: &'a str,
}

/// Build the canonical identity prompt for a provider, optionally prefixed with
/// a mode paragraph.
pub fn build_system_prompt(opts: SystemPromptOpts<'_>) -> String {
    let identity = select_identity_prompt(opts.provider);
    match opts.mode {
        Some(mode) => format!("{}\n\n{}", mode::paragraph(mode), identity),
        None => identity.to_string(),
    }
}

/// Resolve the system prompt for a turn: a non-empty caller override wins;
/// otherwise assemble the built-in identity prompt.
pub fn resolve_system_prompt(
    override_prompt: Option<String>,
    mode: Option<Mode>,
    provider: Option<&str>,
) -> Option<String> {
    match override_prompt.as_deref() {
        Some(s) if !s.is_empty() => Some(s.to_string()),
        _ => Some(build_system_prompt(SystemPromptOpts {
            mode,
            provider: provider.unwrap_or(""),
        })),
    }
}

#[cfg(test)]
mod tests;
