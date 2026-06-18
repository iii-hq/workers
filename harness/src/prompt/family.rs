//! Provider → prompt-family routing (mirrors the legacy turn-orchestrator).

use super::variants;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptFamily {
    Anthropic,
    Gpt,
    Kimi,
    Default,
}

/// Map the run's routed provider id to a prompt family. Routing authority
/// lives in the llm-router; this is a pure lookup with no routing logic.
pub fn prompt_family(provider: &str) -> PromptFamily {
    match provider {
        "anthropic" => PromptFamily::Anthropic,
        "openai" => PromptFamily::Gpt,
        "kimi" => PromptFamily::Kimi,
        // No routed provider (router unreachable during provisioning): mirror the
        // router's seeded default_provider so the un-routed prompt matches what
        // the routed turn would have served.
        "" => PromptFamily::Anthropic,
        _ => PromptFamily::Default,
    }
}

pub fn identity_prompt(family: PromptFamily) -> &'static str {
    match family {
        PromptFamily::Anthropic => variants::ANTHROPIC,
        PromptFamily::Gpt => variants::GPT,
        PromptFamily::Kimi => variants::KIMI,
        PromptFamily::Default => variants::DEFAULT,
    }
}

pub fn select_identity_prompt(provider: &str) -> &'static str {
    identity_prompt(prompt_family(provider))
}
