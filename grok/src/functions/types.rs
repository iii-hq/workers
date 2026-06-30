//! Request payloads. `RunRequest` derives `JsonSchema` so the engine publishes
//! the parameter table for `iii trigger grok::run --help`, and `Deserialize`
//! so the handler parses at the unknown boundary.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Message {
    pub role: String,
    /// Either a plain string or an array of content blocks.
    pub content: Value,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RunRequest {
    /// iii session id; reuse to resume the same Grok thread.
    pub session_id: Option<String>,
    /// The user prompt for this turn.
    pub prompt: Option<String>,
    /// Alternative to prompt: role/content messages; the last user entry becomes the prompt.
    pub messages: Option<Vec<Message>>,
    /// Model id; empty = Grok default (e.g. grok-build-0.1).
    pub model: Option<String>,
    /// Working directory the turn runs in (--cwd).
    pub cwd: Option<String>,
    /// Auto-approve tool/command execution for this headless turn (--always-approve).
    pub always_approve: Option<bool>,
    /// Additional readable/writable directories alongside the working root (--add-dir).
    pub additional_directories: Option<Vec<String>>,
    /// Inject the iii runtime discovery prompt as the leading instructions (default true via config).
    pub iii_context: Option<bool>,
    /// Reserved for callers; not forwarded.
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SessionIdRequest {
    /// iii session id returned by grok::run / grok::start.
    pub session_id: String,
}

/// Extract the prompt: explicit `prompt` (incl. empty string) wins; otherwise
/// the last user message's text.
pub fn extract_prompt(req: &RunRequest) -> anyhow::Result<String> {
    if let Some(p) = &req.prompt {
        return Ok(p.clone());
    }
    let messages = req.messages.as_ref().ok_or_else(|| {
        anyhow::anyhow!("grok::run requires `prompt` or a user message in `messages`")
    })?;
    let last = messages.iter().rfind(|m| m.role == "user").ok_or_else(|| {
        anyhow::anyhow!("grok::run requires `prompt` or a user message in `messages`")
    })?;
    match &last.content {
        Value::String(s) => Ok(s.clone()),
        Value::Array(blocks) => Ok(blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n")),
        _ => Err(anyhow::anyhow!(
            "unsupported message content: expected a string or an array of content blocks"
        )),
    }
}
