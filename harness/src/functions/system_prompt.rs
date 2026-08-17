//! Request-independent preview of the system prompt layers a session will use.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::prompt::{self, Mode, SystemPromptOpts, SystemPromptStrategy};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SelectedSystemPrompt {
    pub name: String,
    pub body: String,
    #[serde(default)]
    pub strategy: SystemPromptStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SystemPromptRequest {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_prompt: Option<SelectedSystemPrompt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem_root: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SystemPromptPartKind {
    BuiltIn,
    Selected,
    Runtime,
    RegistryNotice,
    Injected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SystemPromptPart {
    pub kind: SystemPromptPartKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SystemPromptPreview {
    pub parts: Vec<SystemPromptPart>,
}

pub async fn handle(
    deps: &Deps,
    req: SystemPromptRequest,
) -> Result<SystemPromptPreview, HarnessError> {
    let cfg = deps.cfg().await;
    let record = crate::state::get_turn(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?;
    let provider = req
        .provider
        .as_deref()
        .or_else(|| record.as_ref().and_then(|r| r.options.provider.as_deref()));
    let mode = req
        .mode
        .or_else(|| record.as_ref().and_then(|r| r.options.mode));
    // With no caller-supplied selection, the turn record's RESOLVED prompt is
    // the session's truth: it is what the last turn ran under and — prompt
    // fields omitted — what the next send inherits. A `None` there (a
    // `disabled` turn) means no built-in layer at all. A selection previews a
    // NEW composition instead, so it rebuilds the built-in fresh.
    let frozen = match (&req.selected_prompt, record.as_ref()) {
        (None, Some(record)) => Some(record.options.system_prompt.clone()),
        _ => None,
    };
    let identity = if cfg.provider_identity_prompt && frozen.is_none() {
        deps.router().await.system_prompt_get(provider).await
    } else {
        None
    };
    let (built_in_name, built_in) = match frozen {
        Some(prompt) => ("session (frozen at send)".to_string(), prompt),
        None => (
            built_in_name(provider, identity.as_deref()),
            Some(prompt::build_system_prompt(SystemPromptOpts {
                mode,
                identity: identity.as_deref(),
            })),
        ),
    };

    let filesystem_root = req.filesystem_root.or_else(|| match record.as_ref() {
        Some(record) => record.options.filesystem_root().map(str::to_string),
        None => cfg.resolved_default_filesystem_root(),
    });
    let policy = match record.as_ref() {
        Some(record) => record.options.functions.as_ref(),
        None => cfg.default_functions.as_ref(),
    };
    let runtime =
        crate::turn_loop::runtime_context_aid(&req.session_id, filesystem_root.as_deref(), policy);
    let registry_notice = match record.as_ref() {
        Some(record) => crate::turn_loop::registry_notice(
            record.functions_generation,
            deps.functions().await.generation,
        ),
        None => None,
    };
    let injections = deps
        .hooks
        .pre_generate
        .ordered()
        .into_iter()
        .filter_map(|binding| {
            let body = binding.inject_prompt?;
            Some(SystemPromptPart {
                kind: SystemPromptPartKind::Injected,
                name: Some(worker_name(&binding.function_id)),
                body,
            })
        })
        .collect();

    Ok(SystemPromptPreview {
        parts: build_parts(
            built_in_name,
            built_in,
            req.selected_prompt,
            runtime,
            registry_notice,
            injections,
        ),
    })
}

fn built_in_name(provider: Option<&str>, identity: Option<&str>) -> String {
    match (
        identity.is_some(),
        provider.filter(|provider| !provider.is_empty()),
    ) {
        (true, Some(provider)) => format!("{provider} identity"),
        (true, None) => "provider identity".to_string(),
        (false, _) => "embedded harness default".to_string(),
    }
}

fn worker_name(function_id: &str) -> String {
    function_id
        .split_once("::")
        .map(|(worker, _)| worker)
        .unwrap_or(function_id)
        .to_string()
}

fn build_parts(
    built_in_name: String,
    built_in: Option<String>,
    selected: Option<SelectedSystemPrompt>,
    runtime: String,
    registry_notice: Option<String>,
    injections: Vec<SystemPromptPart>,
) -> Vec<SystemPromptPart> {
    let selected = selected.filter(|prompt| !prompt.body.is_empty());
    let strategy = selected.as_ref().map(|prompt| prompt.strategy);
    let mut parts = Vec::new();
    if let Some(built_in) = built_in {
        if !matches!(
            strategy,
            Some(SystemPromptStrategy::Override | SystemPromptStrategy::Disabled)
        ) {
            parts.push(SystemPromptPart {
                kind: SystemPromptPartKind::BuiltIn,
                name: Some(built_in_name),
                body: built_in,
            });
        }
    }
    if let Some(prompt) =
        selected.filter(|prompt| prompt.strategy != SystemPromptStrategy::Disabled)
    {
        parts.push(SystemPromptPart {
            kind: SystemPromptPartKind::Selected,
            name: Some(prompt.name),
            body: prompt.body,
        });
    }
    parts.push(SystemPromptPart {
        kind: SystemPromptPartKind::Runtime,
        name: None,
        body: runtime,
    });
    if let Some(body) = registry_notice {
        parts.push(SystemPromptPart {
            kind: SystemPromptPartKind::RegistryNotice,
            name: None,
            body,
        });
    }
    parts.extend(injections);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrich_preview_keeps_layers_separate_and_in_send_order() {
        let parts = build_parts(
            "embedded harness default".into(),
            Some("built in".into()),
            Some(SelectedSystemPrompt {
                name: "ptbr".into(),
                body: "selected".into(),
                strategy: SystemPromptStrategy::Enrich,
            }),
            "session context".into(),
            None,
            vec![SystemPromptPart {
                kind: SystemPromptPartKind::Injected,
                name: Some("fp".into()),
                body: "fp guidance".into(),
            }],
        );
        assert_eq!(
            parts.iter().map(|part| part.kind).collect::<Vec<_>>(),
            vec![
                SystemPromptPartKind::BuiltIn,
                SystemPromptPartKind::Selected,
                SystemPromptPartKind::Runtime,
                SystemPromptPartKind::Injected,
            ]
        );
        assert_eq!(parts[3].name.as_deref(), Some("fp"));
    }

    #[test]
    fn override_preview_omits_the_replaced_built_in_layer() {
        let parts = build_parts(
            "embedded harness default".into(),
            Some("built in".into()),
            Some(SelectedSystemPrompt {
                name: "strict".into(),
                body: "selected".into(),
                strategy: SystemPromptStrategy::Override,
            }),
            "session context".into(),
            None,
            Vec::new(),
        );
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].kind, SystemPromptPartKind::Selected);
        assert_eq!(parts[1].kind, SystemPromptPartKind::Runtime);
    }

    #[test]
    fn frozen_record_prompt_replaces_the_rebuilt_built_in() {
        // No caller selection + a turn record → the record's resolved prompt
        // IS the first part, whatever it resolved to at send time.
        let parts = build_parts(
            "session (frozen at send)".into(),
            Some("You are TESTBOT.".into()),
            None,
            "session context".into(),
            None,
            Vec::new(),
        );
        assert_eq!(parts[0].kind, SystemPromptPartKind::BuiltIn);
        assert_eq!(parts[0].name.as_deref(), Some("session (frozen at send)"));
        assert_eq!(parts[0].body, "You are TESTBOT.");

        // A prior `disabled` turn resolved to no prompt: no built-in layer.
        let parts = build_parts(
            "session (frozen at send)".into(),
            None,
            None,
            "session context".into(),
            None,
            Vec::new(),
        );
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].kind, SystemPromptPartKind::Runtime);
    }

    #[test]
    fn built_in_label_names_the_actual_source() {
        assert_eq!(built_in_name(None, None), "embedded harness default");
        assert_eq!(
            built_in_name(Some("anthropic"), None),
            "embedded harness default"
        );
        assert_eq!(
            built_in_name(Some("anthropic"), Some("provider prompt")),
            "anthropic identity"
        );
        assert_eq!(
            built_in_name(None, Some("provider prompt")),
            "provider identity"
        );
    }

    #[test]
    fn worker_name_uses_the_function_namespace() {
        assert_eq!(worker_name("fp::inject-guidance"), "fp");
        assert_eq!(worker_name("bare"), "bare");
    }
}
