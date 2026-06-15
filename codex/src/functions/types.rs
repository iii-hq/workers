//! Request payloads. `RunRequest` derives `JsonSchema` so the engine publishes
//! the parameter table for `iii trigger codex::run --help`, and `Deserialize`
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
    /// iii session id; reuse to resume the same Codex thread.
    pub session_id: Option<String>,
    /// The user prompt for this turn.
    pub prompt: Option<String>,
    /// Alternative to prompt: role/content messages; the last user entry becomes the prompt.
    pub messages: Option<Vec<Message>>,
    /// Model id; empty = Codex default.
    pub model: Option<String>,
    /// Working directory the turn runs in.
    pub cwd: Option<String>,
    /// Codex sandbox mode: read-only | workspace-write | danger-full-access.
    pub sandbox_mode: Option<String>,
    /// Codex approval policy; headless callers leave it at never.
    pub approval_policy: Option<String>,
    /// Model reasoning effort: minimal | low | medium | high | xhigh.
    pub reasoning_effort: Option<String>,
    /// Allow running outside a git repository.
    pub skip_git_repo_check: Option<bool>,
    /// JSON schema for structured final output.
    pub output_schema: Option<Value>,
    /// Paths to local images attached to the prompt.
    pub images: Option<Vec<String>>,
    /// Additional writable directories alongside the working root (--add-dir).
    pub additional_directories: Option<Vec<String>>,
    /// Codex config.toml overrides for this turn (e.g. mcp_servers, model_providers, profiles).
    pub codex_config: Option<Value>,
    /// Inject the iii runtime discovery prompt as developer_instructions (default true via config).
    pub iii_context: Option<bool>,
    /// Reserved for callers; not forwarded.
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SessionIdRequest {
    /// iii session id returned by codex::run / codex::start.
    pub session_id: String,
}

/// Extract the prompt: explicit `prompt` (incl. empty string) wins; otherwise
/// the last user message's text.
pub fn extract_prompt(req: &RunRequest) -> anyhow::Result<String> {
    if let Some(p) = &req.prompt {
        return Ok(p.clone());
    }
    let messages = req.messages.as_ref().ok_or_else(|| {
        anyhow::anyhow!("codex::run requires `prompt` or a user message in `messages`")
    })?;
    let last = messages.iter().rfind(|m| m.role == "user").ok_or_else(|| {
        anyhow::anyhow!("codex::run requires `prompt` or a user message in `messages`")
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
