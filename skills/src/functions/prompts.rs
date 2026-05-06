//! State-backed prompts registry, ported from the `mcp` worker.
//!
//! Public API (reachable by any worker over `iii.trigger`):
//!
//!   * `prompts::register`   — store a slash-command prompt definition.
//!   * `prompts::unregister` — delete one by name (idempotent).
//!   * `prompts::list`       — metadata-only listing, sorted by name.
//!
//! Internal RPC called only by the `mcp` worker (hard-floored under
//! `prompts::*` so never an MCP tool):
//!
//!   * `prompts::mcp-list` — `{ prompts: [...] }` for MCP `prompts/list`.
//!   * `prompts::mcp-get`  — `{ description, messages: [...] }` for MCP `prompts/get`.
//!
//! Each mutation fans out through the `prompts::on-change` trigger type
//! (see [`crate::trigger_types`]) so interested workers (`mcp` today)
//! can forward MCP notifications.

use std::collections::HashSet;
use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, TriggerRequest, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::SkillsConfig;
use crate::functions::skills::is_always_hidden;
use crate::state;
use crate::trigger_types::{self, SubscriberSet};

const NAME_MAX_LEN: usize = 64;

#[derive(Debug, Deserialize, JsonSchema)]
struct RegisterPromptInput {
    /// Unique prompt name (lowercase ASCII, kebab/underscore, max 64 chars).
    name: String,
    /// Free-text description shown in the client's prompt picker.
    description: String,
    #[serde(default)]
    arguments: Vec<PromptArgument>,
    /// Handler function called on prompts/get with the supplied arguments.
    function_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct RegisterPromptOutput {
    name: String,
    registered_at: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UnregisterPromptInput {
    name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct UnregisterPromptOutput {
    name: String,
    removed: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ListPromptsInput {}

#[derive(Debug, Serialize, JsonSchema)]
struct PromptEntry {
    name: String,
    function_id: String,
    arguments: usize,
    registered_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ListPromptsOutput {
    prompts: Vec<PromptEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone, JsonSchema)]
pub struct PromptArgument {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StoredPrompt {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub arguments: Vec<PromptArgument>,
    pub function_id: String,
    pub registered_at: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct EmptyInput {}

#[derive(Debug, Deserialize, JsonSchema)]
struct PromptGetInput {
    name: String,
    #[serde(default)]
    arguments: Option<Value>,
}

pub fn register(iii: &Arc<III>, cfg: &Arc<SkillsConfig>, subscribers: &SubscriberSet) {
    register_register_prompt(iii, cfg, subscribers);
    register_unregister_prompt(iii, cfg, subscribers);
    register_list_prompts(iii, cfg);
    register_mcp_list(iii, cfg);
    register_mcp_get(iii, cfg);
}

fn register_register_prompt(iii: &Arc<III>, cfg: &Arc<SkillsConfig>, subscribers: &SubscriberSet) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    let subs_inner = subscribers.clone();
    iii.register_function(
        RegisterFunction::new_async("prompts::register", move |req: RegisterPromptInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            let subs = subs_inner.clone();
            async move {
                validate_name(&req.name).map_err(IIIError::Handler)?;
                if req.description.trim().is_empty() {
                    return Err(IIIError::Handler("description must be non-empty".into()));
                }
                if req.function_id.trim().is_empty() {
                    return Err(IIIError::Handler("function_id must be non-empty".into()));
                }
                validate_arguments(&req.arguments).map_err(IIIError::Handler)?;

                let stored = StoredPrompt {
                    name: req.name.clone(),
                    description: req.description,
                    arguments: req.arguments,
                    function_id: req.function_id,
                    registered_at: chrono::Utc::now().to_rfc3339(),
                };

                let value = serde_json::to_value(&stored)
                    .map_err(|e| IIIError::Handler(format!("encode prompt: {e}")))?;
                state::state_set(
                    &iii,
                    &cfg.scopes.prompts,
                    &req.name,
                    value,
                    cfg.state_timeout_ms,
                )
                .await?;

                tracing::info!(prompt = %req.name, "prompt registered");

                trigger_types::dispatch(&iii, &subs, json!({ "op": "register", "name": req.name }))
                    .await;

                Ok::<_, IIIError>(RegisterPromptOutput {
                    name: req.name,
                    registered_at: stored.registered_at,
                })
            }
        })
        .description("Register a slash-command prompt; clients call prompts/get to render it."),
    );
}

fn register_unregister_prompt(
    iii: &Arc<III>,
    cfg: &Arc<SkillsConfig>,
    subscribers: &SubscriberSet,
) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    let subs_inner = subscribers.clone();
    iii.register_function(
        RegisterFunction::new_async("prompts::unregister", move |req: UnregisterPromptInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            let subs = subs_inner.clone();
            async move {
                validate_name(&req.name).map_err(IIIError::Handler)?;
                let prior =
                    state::state_delete(&iii, &cfg.scopes.prompts, &req.name, cfg.state_timeout_ms)
                        .await?;
                let removed = !prior.is_null();
                tracing::info!(prompt = %req.name, removed, "prompt unregister");

                if removed {
                    trigger_types::dispatch(
                        &iii,
                        &subs,
                        json!({ "op": "unregister", "name": req.name }),
                    )
                    .await;
                }

                Ok::<_, IIIError>(UnregisterPromptOutput {
                    name: req.name,
                    removed,
                })
            }
        })
        .description("Remove a registered prompt by name."),
    );
}

fn register_list_prompts(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async("prompts::list", move |_input: ListPromptsInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            async move {
                let entries = list_stored(&iii, &cfg).await?;
                let mut out: Vec<PromptEntry> = entries
                    .into_iter()
                    .map(|p| PromptEntry {
                        arguments: p.arguments.len(),
                        name: p.name,
                        function_id: p.function_id,
                        registered_at: p.registered_at,
                    })
                    .collect();
                out.sort_by(|a, b| a.name.cmp(&b.name));
                Ok::<_, IIIError>(ListPromptsOutput { prompts: out })
            }
        })
        .description("List registered prompts (name, function_id, arg count, registered_at)."),
    );
}

fn register_mcp_list(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async("prompts::mcp-list", move |_input: EmptyInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            async move { Ok::<_, IIIError>(mcp_list(&iii, &cfg).await) }
        })
        .description(
            "Internal: returns the MCP prompts/list envelope (full arguments schema for each registered prompt).",
        ),
    );
}

fn register_mcp_get(iii: &Arc<III>, cfg: &Arc<SkillsConfig>) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    iii.register_function(
        RegisterFunction::new_async("prompts::mcp-get", move |req: PromptGetInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            async move {
                mcp_get(&iii, &cfg, req.name, req.arguments)
                    .await
                    .map_err(IIIError::Handler)
            }
        })
        .description(
            "Internal: dispatches a registered prompt handler and normalizes the result for MCP prompts/get.",
        ),
    );
}

// ---------- core helpers (reusable in tests) ----------

pub async fn mcp_list(iii: &III, cfg: &SkillsConfig) -> Value {
    let entries = list_stored(iii, cfg).await.unwrap_or_default();
    let mut prompts: Vec<Value> = entries
        .into_iter()
        .map(|p| {
            let arguments: Vec<Value> = p
                .arguments
                .iter()
                .map(|a| {
                    let mut obj = json!({ "name": a.name, "required": a.required });
                    if let Some(d) = &a.description {
                        obj["description"] = Value::String(d.clone());
                    }
                    obj
                })
                .collect();
            json!({
                "name": p.name,
                "description": p.description,
                "arguments": arguments,
            })
        })
        .collect();
    prompts.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    json!({ "prompts": prompts })
}

pub async fn mcp_get(
    iii: &III,
    cfg: &SkillsConfig,
    name: String,
    arguments: Option<Value>,
) -> Result<Value, String> {
    validate_name(&name)?;
    let arguments = arguments.unwrap_or_else(|| json!({}));

    let stored = read_prompt(iii, cfg, &name)
        .await?
        .ok_or_else(|| format!("Prompt not found: {name}"))?;

    if is_always_hidden(&stored.function_id) {
        return Err(format!(
            "Prompt handler '{}' is in an internal namespace and cannot back a prompt",
            stored.function_id
        ));
    }

    let raw = iii
        .trigger(TriggerRequest {
            function_id: stored.function_id.clone(),
            payload: arguments,
            action: None,
            timeout_ms: Some(cfg.state_timeout_ms),
        })
        .await
        .map_err(|e| format!("trigger {}: {e}", stored.function_id))?;

    let messages = normalize_prompt_output(raw)?;
    Ok(json!({ "description": stored.description, "messages": messages }))
}

// ---------- validation ----------

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name must be non-empty".into());
    }
    if name.len() > NAME_MAX_LEN {
        return Err(format!(
            "name too long ({} chars; max {NAME_MAX_LEN})",
            name.len()
        ));
    }
    for c in name.chars() {
        let ok = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
        if !ok {
            return Err(format!(
                "name may only contain lowercase ASCII letters, digits, '-' and '_': {name:?}"
            ));
        }
    }
    Ok(())
}

pub fn validate_arguments(args: &[PromptArgument]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for a in args {
        if a.name.trim().is_empty() {
            return Err("argument name must be non-empty".into());
        }
        if !seen.insert(a.name.clone()) {
            return Err(format!("duplicate argument name: {}", a.name));
        }
    }
    Ok(())
}

// ---------- output normalization ----------

pub fn normalize_prompt_output(raw: Value) -> Result<Value, String> {
    if let Value::String(s) = &raw {
        return Ok(json!([single_user_message(s)]));
    }
    if let Some(messages) = raw.get("messages").and_then(|v| v.as_array()) {
        return Ok(Value::Array(messages.clone()));
    }
    if let Some(content) = raw.get("content").and_then(|v| v.as_str()) {
        return Ok(json!([single_user_message(content)]));
    }
    Err("prompt handler returned unsupported shape; expected string, { content }, or { messages: [...] }".into())
}

fn single_user_message(text: &str) -> Value {
    json!({
        "role": "user",
        "content": { "type": "text", "text": text }
    })
}

// ---------- state wrappers ----------

async fn read_prompt(
    iii: &III,
    cfg: &SkillsConfig,
    name: &str,
) -> Result<Option<StoredPrompt>, String> {
    let raw = state::state_get(iii, &cfg.scopes.prompts, name, cfg.state_timeout_ms)
        .await
        .map_err(|e| format!("state::get: {e}"))?;
    if raw.is_null() {
        return Ok(None);
    }
    serde_json::from_value::<StoredPrompt>(raw)
        .map(Some)
        .map_err(|e| format!("decode stored prompt {name}: {e}"))
}

async fn list_stored(iii: &III, cfg: &SkillsConfig) -> Result<Vec<StoredPrompt>, IIIError> {
    let raw = state::state_list(iii, &cfg.scopes.prompts, cfg.state_timeout_ms).await?;
    let entries = state::extract_state_entries(raw);
    let mut out: Vec<StoredPrompt> = entries
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validation_accepts_kebab_and_underscore() {
        assert!(validate_name("send-email").is_ok());
        assert!(validate_name("triage_ticket").is_ok());
        assert!(validate_name("a").is_ok());
        assert!(validate_name("v2").is_ok());
    }

    #[test]
    fn name_validation_rejects_bad_chars() {
        assert!(validate_name("").is_err());
        assert!(validate_name("Send-Email").is_err());
        assert!(validate_name("send email").is_err());
        assert!(validate_name("send/email").is_err());
        assert!(validate_name("mcp::send").is_err());
        assert!(validate_name(&"x".repeat(NAME_MAX_LEN + 1)).is_err());
    }

    #[test]
    fn argument_validation_rejects_duplicates() {
        let args = vec![
            PromptArgument {
                name: "to".into(),
                description: None,
                required: true,
            },
            PromptArgument {
                name: "to".into(),
                description: None,
                required: false,
            },
        ];
        assert!(validate_arguments(&args).is_err());
    }

    #[test]
    fn argument_validation_rejects_empty_names() {
        let args = vec![PromptArgument {
            name: "".into(),
            description: None,
            required: true,
        }];
        assert!(validate_arguments(&args).is_err());
    }

    #[test]
    fn argument_validation_accepts_unique_names() {
        let args = vec![
            PromptArgument {
                name: "to".into(),
                description: None,
                required: true,
            },
            PromptArgument {
                name: "subject".into(),
                description: None,
                required: false,
            },
        ];
        assert!(validate_arguments(&args).is_ok());
    }

    #[test]
    fn normalize_string_wraps_as_user_message() {
        let out = normalize_prompt_output(Value::String("hello".into())).unwrap();
        let msgs = out.as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"]["type"], "text");
        assert_eq!(msgs[0]["content"]["text"], "hello");
    }

    #[test]
    fn normalize_content_wraps_as_user_message() {
        let out = normalize_prompt_output(json!({ "content": "hi" })).unwrap();
        let msgs = out.as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["content"]["text"], "hi");
    }

    #[test]
    fn normalize_messages_passes_through() {
        let raw = json!({
            "messages": [
                { "role": "user", "content": { "type": "text", "text": "a" } },
                { "role": "assistant", "content": { "type": "text", "text": "b" } }
            ]
        });
        let out = normalize_prompt_output(raw).unwrap();
        let msgs = out.as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
    }

    #[test]
    fn normalize_other_returns_error() {
        let err = normalize_prompt_output(json!({ "x": 1 })).unwrap_err();
        assert!(err.contains("unsupported shape"));
    }
}
