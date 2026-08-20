//! Secondary LLM composition through `llm-router`.
//!
//! The Harness model supplies compact intent. This module asks a separate
//! composition turn for protocol messages, parses JSON/JSONL, validates the
//! complete surface against the iii Console catalog, and performs a bounded
//! correction pass before anything reaches durable state.

use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::WorkerConfig;
use crate::protocol::{
    apply_messages, ensure_interactive_data_submission, validate_renderable, ServerMessage,
    SessionState, SurfaceRecord, PROTOCOL_VERSION,
};

pub struct ComposeInput<'a> {
    pub session_id: &'a str,
    pub surface_id: &'a str,
    pub description: &'a str,
    pub data: Option<&'a Value>,
    pub existing_surface: Option<&'a SurfaceRecord>,
    pub inherited_model: Option<&'a str>,
    pub inherited_provider: Option<&'a str>,
}

pub struct Composer {
    iii: Arc<IIIClient>,
}

const COMPOSE_TOOL_NAME: &str = "submit_a2ui";

#[derive(Deserialize, JsonSchema)]
struct CompositionSubmission {
    messages: Vec<ServerMessage>,
}

impl Composer {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }

    pub async fn compose(
        &self,
        input: ComposeInput<'_>,
        cfg: &WorkerConfig,
    ) -> Result<Vec<ServerMessage>, String> {
        let turn_routing = if cfg.composer_model.is_none() && input.inherited_model.is_none() {
            Some(self.load_turn_routing(input.session_id).await?)
        } else {
            None
        };
        let model = cfg
            .composer_model
            .as_deref()
            .or(input.inherited_model)
            .or_else(|| turn_routing.as_ref().map(|routing| routing.model.as_str()))
            .filter(|model| !model.trim().is_empty())
            .ok_or_else(|| {
                "no composer model is available; call from Harness or set composer_model"
                    .to_string()
            })?;
        let provider = cfg
            .composer_provider
            .as_deref()
            .or(input.inherited_provider)
            .or_else(|| {
                turn_routing
                    .as_ref()
                    .and_then(|routing| routing.provider.as_deref())
            })
            .filter(|provider| !provider.trim().is_empty());

        let data = input
            .data
            .map(|value| serde_json::to_string(value).unwrap_or_else(|_| "null".into()))
            .unwrap_or_else(|| "null".into());
        let user_prompt = if let Some(surface) = input.existing_surface {
            let current = json!({
                "title": surface.title,
                "theme": surface.theme,
                "sendDataModel": surface.send_data_model,
                "components": surface.components,
                "dataModel": surface.data_model,
                "revision": surface.revision,
            });
            let current = serde_json::to_string(&current)
                .unwrap_or_else(|_| "unable to serialize current surface".into());
            format!(
                "Replace the A2UI surface `{}` with a complete updated version. Preserve everything not requested.\n\nChange request:\n{}\n\nCurrent surface:\n{}\n\nAdditional data or overrides:\n{}",
                input.surface_id, input.description, current, data
            )
        } else {
            format!(
                "Create the A2UI surface `{}` for this request:\n\n{}\n\nSeed data:\n{}",
                input.surface_id, input.description, data
            )
        };
        if user_prompt.len() > cfg.max_composer_input_bytes {
            return Err(format!(
                "composer input is {} bytes; maximum is {}",
                user_prompt.len(),
                cfg.max_composer_input_bytes
            ));
        }
        let mut messages = vec![json!({
            "role": "user",
            "content": [{"type": "text", "text": user_prompt}],
            "timestamp": crate::protocol::now_ms(),
        })];
        let mut last_error = String::new();

        for attempt in 0..=cfg.repair_attempts {
            if attempt > 0 {
                messages.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "text",
                        "text": format!(
                            "Your previous response was rejected: {last_error}. Return a corrected JSON array only."
                        )
                    }],
                    "timestamp": crate::protocol::now_ms(),
                }));
            }
            let output = self
                .complete(
                    input.session_id,
                    model,
                    provider,
                    messages.clone(),
                    cfg.max_output_tokens,
                )
                .await?;
            match parse_and_validate(&output, input.session_id, input.surface_id, cfg) {
                Ok(parsed) => return Ok(parsed),
                Err(error) => {
                    last_error = error;
                    messages.push(json!({
                        "role": "assistant",
                        "content": [{"type": "text", "text": output}],
                        "stop_reason": "end",
                        "native_stop_reason": null,
                        "error_message": null,
                        "error_kind": null,
                        "warnings": null,
                        "usage": null,
                        "model": model,
                        "provider": provider.unwrap_or("router"),
                        "timestamp": crate::protocol::now_ms(),
                    }));
                }
            }
        }
        Err(format!(
            "A2UI composition stayed invalid after {} attempt(s): {last_error}",
            usize::from(cfg.repair_attempts) + 1
        ))
    }

    async fn load_turn_routing(&self, session_id: &str) -> Result<TurnRouting, String> {
        let record = self
            .iii
            .trigger(TriggerRequest {
                function_id: "state::get".into(),
                payload: json!({"scope": "harness_turn", "key": session_id}),
                action: None,
                timeout_ms: Some(10_000),
            })
            .await
            .map_err(|error| format!("could not read Harness turn routing: {error}"))?;
        turn_routing(&record).ok_or_else(|| {
            "the active Harness turn has no model; set composer_model in A2UI configuration"
                .to_string()
        })
    }

    async fn complete(
        &self,
        session_id: &str,
        model: &str,
        provider: Option<&str>,
        messages: Vec<Value>,
        max_output_tokens: u64,
    ) -> Result<String, String> {
        let mut payload = json!({
            "session_id": format!("a2ui:{session_id}"),
            "model": model,
            "system_prompt": SYSTEM_PROMPT,
            "messages": messages,
            "tools": [{
                "name": COMPOSE_TOOL_NAME,
                "description": "Submit the complete validated A2UI server-message batch.",
                "parameters": schema_for!(CompositionSubmission),
            }],
            "max_output_tokens": max_output_tokens,
            "metadata": {"purpose": "a2ui-composition", "protocol_version": PROTOCOL_VERSION},
        });
        if let Some(provider) = provider {
            payload["provider"] = json!(provider);
        }
        let response = self
            .iii
            .trigger(TriggerRequest {
                function_id: "router::complete".into(),
                payload,
                action: None,
                timeout_ms: Some(600_000),
            })
            .await
            .map_err(|error| format!("router::complete failed: {error}"))?;
        extract_composition(&response)
    }
}

struct TurnRouting {
    model: String,
    provider: Option<String>,
}

fn turn_routing(record: &Value) -> Option<TurnRouting> {
    let options = record.get("options")?;
    let model = options.get("model")?.as_str()?.trim();
    if model.is_empty() {
        return None;
    }
    let provider = options
        .get("provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Some(TurnRouting {
        model: model.to_string(),
        provider,
    })
}

fn extract_composition(response: &Value) -> Result<String, String> {
    let blocks = response
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
        .ok_or_else(|| "router::complete returned no assistant content".to_string())?;
    if let Some(arguments) = blocks.iter().find_map(|block| {
        (block.get("type").and_then(Value::as_str) == Some("function_call")
            && block.get("function_id").and_then(Value::as_str) == Some(COMPOSE_TOOL_NAME))
        .then(|| block.get("arguments"))
        .flatten()
    }) {
        let submission: CompositionSubmission = serde_json::from_value(arguments.clone())
            .map_err(|error| format!("composer tool arguments were invalid: {error}"))?;
        return serde_json::to_string(&submission.messages)
            .map_err(|error| format!("could not serialize composer messages: {error}"));
    }
    let text = blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        Err(format!(
            "router::complete returned neither a `{COMPOSE_TOOL_NAME}` call nor a text block"
        ))
    } else {
        Ok(text)
    }
}

pub fn parse_messages(raw: &str) -> Result<Vec<ServerMessage>, String> {
    let text = strip_fence(raw.trim());
    if let Ok(messages) = serde_json::from_str::<Vec<ServerMessage>>(text) {
        return Ok(messages);
    }
    if let Ok(object) = serde_json::from_str::<Value>(text) {
        if let Some(messages) = object.get("messages") {
            return serde_json::from_value(messages.clone())
                .map_err(|error| format!("invalid messages array: {error}"));
        }
        if let Ok(message) = serde_json::from_value::<ServerMessage>(object) {
            return Ok(vec![message]);
        }
    }
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_str::<ServerMessage>(line)
                .map_err(|error| format!("invalid JSONL message {}: {error}", index + 1))
        })
        .collect()
}

fn parse_and_validate(
    raw: &str,
    session_id: &str,
    surface_id: &str,
    cfg: &WorkerConfig,
) -> Result<Vec<ServerMessage>, String> {
    let mut messages = parse_messages(raw)?;
    if messages
        .iter()
        .any(|message| message.surface_id() != surface_id)
    {
        return Err(format!(
            "composer changed the required surfaceId `{surface_id}`"
        ));
    }
    ensure_interactive_data_submission(&mut messages);
    let mut state = SessionState::empty(session_id);
    apply_messages(&mut state, &messages, None, cfg)?;
    let surface = state
        .get(surface_id)
        .ok_or_else(|| "composer deleted the generated surface".to_string())?;
    validate_renderable(surface)?;
    Ok(messages)
}

fn strip_fence(text: &str) -> &str {
    let Some(after) = text.strip_prefix("```") else {
        return text;
    };
    let after = after
        .strip_prefix("json")
        .or_else(|| after.strip_prefix("jsonl"))
        .unwrap_or(after)
        .trim_start_matches(['\r', '\n']);
    after.strip_suffix("```").map(str::trim).unwrap_or(after)
}

const SYSTEM_PROMPT: &str = r#"You compose safe declarative A2UI. You MUST call the submit_a2ui tool exactly once with the complete batch. Do not return prose.

Every message must use version "v0.9.1" and the exact requested surfaceId. Emit in this order:
1. createSurface with catalogId "urn:iii:a2ui:console:v0.1".
2. updateComponents with a flat adjacency list containing exactly one component with id "root".
3. updateDataModel at path "/" when seed data or bindings are useful.

When the surface contains TextField or CheckBox inputs, createSurface must set sendDataModel to true so actions include the user's complete form values. For an update request, still emit a complete replacement batch containing createSurface, the full component graph, and the full data model. Preserve current content and values unless the change request says otherwise.

Supported catalog components and properties:
- Column: children: string[], gap?: "sm"|"md"|"lg", align?: "start"|"center"|"end"|"stretch"
- Row: children: string[], gap?: "sm"|"md"|"lg", align?: "start"|"center"|"end"|"stretch", wrap?: boolean
- Card: child: string
- Text: text: string or {"path":"/json/pointer"}, variant?: "h1"|"h2"|"body"|"caption"
- Badge: text: string or path binding, variant?: "default"|"accent"|"warn"|"alert"
- Button: child: string, variant?: "primary"|"ghost", action: {"event":{"name":string,"context"?:object}}
- TextField: label: string, value: {"path":"/json/pointer"}, placeholder?: string
- CheckBox: label: string, value: {"path":"/json/pointer"}
- Divider: no additional properties

Use only these components. All child/children ids must exist. The graph must be acyclic. Keep content concise. Bind changing/input values through JSON Pointer paths instead of copying data into component structure. Never emit HTML, JavaScript, CSS, URLs, iframe/embed instructions, or executable code."#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_array_jsonl_and_fenced_json() {
        let create = format!(
            r#"{{"version":"{PROTOCOL_VERSION}","createSurface":{{"surfaceId":"main","catalogId":"{}"}}}}"#,
            crate::protocol::CATALOG_ID
        );
        assert_eq!(parse_messages(&format!("[{create}]")).unwrap().len(), 1);
        assert_eq!(parse_messages(&format!("{create}\n")).unwrap().len(), 1);
        assert_eq!(
            parse_messages(&format!("```json\n[{create}]\n```"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn extracts_tool_submission_and_falls_back_to_text_content() {
        let response = json!({"message": {"content": [{
            "type": "function_call",
            "id": "call-1",
            "function_id": COMPOSE_TOOL_NAME,
            "arguments": {"messages": [{
                "version": PROTOCOL_VERSION,
                "createSurface": {"surfaceId": "main", "catalogId": crate::protocol::CATALOG_ID}
            }]}
        }]}});
        assert_eq!(
            parse_messages(&extract_composition(&response).unwrap())
                .unwrap()
                .len(),
            1
        );

        let response = json!({"message": {"content": [
            {"type": "thinking", "text": "private"},
            {"type": "text", "text": "public"}
        ]}});
        assert_eq!(extract_composition(&response).unwrap(), "public");
    }

    #[test]
    fn reads_model_and_provider_from_harness_turn_state() {
        let routing = turn_routing(&json!({
            "options": {"model": "model-a", "provider": "provider-a"}
        }))
        .unwrap();
        assert_eq!(routing.model, "model-a");
        assert_eq!(routing.provider.as_deref(), Some("provider-a"));
        assert!(turn_routing(&json!({"options": {"model": ""}})).is_none());
    }
}
