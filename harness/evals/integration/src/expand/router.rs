use std::collections::BTreeMap;

use anyhow::Context;
use serde_json::{json, Value};

use crate::types::frames::{
    AssistantMessage, AssistantMessageEvent, AssistantRoleTag, ContentBlock, RouterChatResponse,
    StopReason,
};
use crate::types::scenario::{IntegrationScenarioV1, RouterReplyV1};
use crate::types::script::{
    GenerationMatchV1, JsonMatcherV1, JsonNormalizerV1, ModelFixtureV1, NormalizerOperation,
    RouterScriptV1, SchemaVersion1, ScriptedGenerationV1,
};

use super::{validation::validate_reply, CompiledFunctionCall};

struct CompiledReply {
    frames: Vec<AssistantMessageEvent>,
    response: RouterChatResponse,
    history: Vec<Value>,
}

pub(super) fn compile_router(
    authored: &IntegrationScenarioV1,
    model: &ModelFixtureV1,
    tools: &Value,
    function_ids: &BTreeMap<String, String>,
    calls: &[CompiledFunctionCall],
) -> anyhow::Result<RouterScriptV1> {
    let mut messages = vec![json!({
        "role": "user",
        "content": [{ "type": "text", "text": authored.send.message }]
    })];
    let mut generations = Vec::with_capacity(authored.router.generations.len());

    for (index, authored_generation) in authored.router.generations.iter().enumerate() {
        let ordinal = (index + 1) as u64;
        let call_id = if matches!(
            &authored_generation.reply,
            RouterReplyV1::FunctionCall { .. }
        ) {
            let id = calls
                .iter()
                .find(|call| call.generation_index == index && call.typed)
                .context("compiler lost a typed function-call id")?;
            Some(id.id.as_str())
        } else {
            None
        };
        validate_reply(&authored_generation.reply, ordinal, authored, function_ids)?;
        let CompiledReply {
            frames,
            response,
            history,
        } = compile_reply(
            &authored_generation.reply,
            ordinal,
            model,
            authored,
            function_ids,
            call_id,
        )?;
        let mut match_ = default_match(ordinal, model, &messages, tools);
        apply_match_overrides(&mut match_, &authored_generation.match_overrides);
        generations.push(ScriptedGenerationV1 {
            ordinal,
            match_,
            frames,
            response,
        });
        messages.extend(history);
    }

    Ok(RouterScriptV1 {
        schema_version: SchemaVersion1::V1,
        scenario_id: authored.id.clone(),
        model: model.clone(),
        generations,
    })
}

fn default_match(
    ordinal: u64,
    model: &ModelFixtureV1,
    messages: &[Value],
    tools: &Value,
) -> GenerationMatchV1 {
    let normalize = (0..messages.len())
        .map(|index| JsonNormalizerV1 {
            pointer: format!("/{index}/timestamp"),
            operation: NormalizerOperation::Delete,
            replacement: None,
        })
        .collect();
    GenerationMatchV1 {
        writer_ref: JsonMatcherV1::Subset {
            expected: json!({ "direction": "write" }),
            normalize: None,
        },
        request_id: JsonMatcherV1::Regex {
            pattern: if ordinal == 1 {
                "^t_[0-9a-f]{32}:[0-9]+$".to_string()
            } else {
                format!("^t_[0-9a-f]{{32}}:{}$", ordinal - 1)
            },
        },
        model: JsonMatcherV1::Exact {
            expected: json!(model.id),
            normalize: None,
        },
        provider: JsonMatcherV1::Exact {
            expected: json!(model.provider),
            normalize: None,
        },
        system_prompt: JsonMatcherV1::Sha256 {
            expected: "{{system_prompt_sha256}}".to_string(),
        },
        messages: JsonMatcherV1::Exact {
            expected: Value::Array(messages.to_vec()),
            normalize: Some(normalize),
        },
        tools: JsonMatcherV1::Exact {
            expected: tools.clone(),
            normalize: None,
        },
        response_format: JsonMatcherV1::Absent,
        thinking_level: JsonMatcherV1::Absent,
        max_output_tokens: JsonMatcherV1::Absent,
        provider_options: JsonMatcherV1::Absent,
        metadata: JsonMatcherV1::Absent,
    }
}

fn apply_match_overrides(
    target: &mut GenerationMatchV1,
    overrides: &crate::types::scenario::GenerationMatchOverridesV1,
) {
    macro_rules! replace {
        ($field:ident) => {
            if let Some(value) = &overrides.$field {
                target.$field = value.clone();
            }
        };
    }
    replace!(writer_ref);
    replace!(request_id);
    replace!(model);
    replace!(provider);
    replace!(system_prompt);
    replace!(messages);
    replace!(tools);
    replace!(response_format);
    replace!(thinking_level);
    replace!(max_output_tokens);
    replace!(provider_options);
    replace!(metadata);
}

fn compile_reply(
    reply: &RouterReplyV1,
    ordinal: u64,
    model: &ModelFixtureV1,
    authored: &IntegrationScenarioV1,
    function_ids: &BTreeMap<String, String>,
    call_id: Option<&str>,
) -> anyhow::Result<CompiledReply> {
    match reply {
        RouterReplyV1::Text {
            text,
            chunks,
            usage,
        } => {
            let message = assistant_message(
                vec![ContentBlock::Text { text: text.clone() }],
                StopReason::End,
                usage.clone(),
                model,
                ordinal as i64,
            );
            let mut frames = Vec::new();
            if chunks.is_empty() {
                frames.push(AssistantMessageEvent::Done {
                    message: message.clone(),
                });
            } else {
                frames.push(AssistantMessageEvent::Start {
                    partial: assistant_message(
                        Vec::new(),
                        StopReason::End,
                        None,
                        model,
                        ordinal as i64,
                    ),
                });
                frames.push(AssistantMessageEvent::TextStart {
                    partial: assistant_message(
                        vec![ContentBlock::Text {
                            text: String::new(),
                        }],
                        StopReason::End,
                        None,
                        model,
                        ordinal as i64,
                    ),
                });
                frames.extend(chunks.iter().cloned().map(|delta| {
                    AssistantMessageEvent::TextDelta {
                        partial: None,
                        delta,
                    }
                }));
                frames.push(AssistantMessageEvent::TextEnd {
                    partial: assistant_message(
                        vec![ContentBlock::Text { text: text.clone() }],
                        StopReason::End,
                        None,
                        model,
                        ordinal as i64,
                    ),
                });
                if let Some(usage) = usage {
                    frames.push(AssistantMessageEvent::Usage {
                        usage: usage.clone(),
                    });
                }
                frames.push(AssistantMessageEvent::Stop {
                    stop_reason: StopReason::End,
                    error_message: None,
                    error_kind: None,
                });
                frames.push(AssistantMessageEvent::Done {
                    message: message.clone(),
                });
            }
            Ok(CompiledReply {
                frames,
                response: RouterChatResponse {
                    ok: true,
                    provider: model.provider.clone(),
                    model: model.id.clone(),
                    stop_reason: Some(StopReason::End),
                    usage: usage.clone(),
                    error: None,
                },
                history: vec![json!({
                    "role": "assistant",
                    "content": [{ "type": "text", "text": text }],
                    "stop_reason": "end",
                    "model": model.id,
                    "provider": model.provider
                })],
            })
        }
        RouterReplyV1::FunctionCall {
            function,
            arguments,
            usage,
            ..
        } => {
            let call_id = call_id.context("missing compiled function-call id")?;
            let message = assistant_message(
                vec![ContentBlock::FunctionCall {
                    id: call_id.to_string(),
                    function_id: function_ids[function].clone(),
                    arguments: arguments.clone(),
                }],
                StopReason::FunctionCall,
                usage.clone(),
                model,
                ordinal as i64,
            );
            let function_id = &function_ids[function];
            let function_response = &authored.functions[function].response;
            let (content, is_error) = normalize_function_response(function_response);
            Ok(CompiledReply {
                frames: vec![AssistantMessageEvent::Done { message }],
                response: RouterChatResponse {
                    ok: true,
                    provider: model.provider.clone(),
                    model: model.id.clone(),
                    stop_reason: Some(StopReason::FunctionCall),
                    usage: usage.clone(),
                    error: None,
                },
                history: vec![
                    json!({
                        "role": "assistant",
                        "content": [{
                            "type": "function_call",
                            "id": call_id,
                            "function_id": function_id,
                            "arguments": arguments
                        }],
                        // The harness persists an open call with its default
                        // stop reason; the wire response remains
                        // `function_call`.
                        "stop_reason": "end",
                        "model": model.id,
                        "provider": model.provider
                    }),
                    json!({
                        "role": "function_result",
                        "function_call_id": call_id,
                        "function_id": function_id,
                        "content": content,
                        "details": function_response,
                        "is_error": is_error
                    }),
                ],
            })
        }
        RouterReplyV1::Raw { frames, response } => {
            let history = match frames.last() {
                Some(AssistantMessageEvent::Done { message }) => {
                    vec![serde_json::to_value(message)?]
                }
                _ => Vec::new(),
            };
            Ok(CompiledReply {
                frames: frames.clone(),
                response: response.clone(),
                history,
            })
        }
    }
}

fn assistant_message(
    content: Vec<ContentBlock>,
    stop_reason: StopReason,
    usage: Option<crate::types::frames::Usage>,
    model: &ModelFixtureV1,
    timestamp: i64,
) -> AssistantMessage {
    AssistantMessage {
        role: AssistantRoleTag::Assistant,
        content,
        stop_reason,
        native_stop_reason: None,
        error_message: None,
        error_kind: None,
        warnings: None,
        usage,
        model: model.id.clone(),
        provider: model.provider.clone(),
        timestamp,
    }
}

/// Mirrors `harness::trigger::normalize`: the next router generation must
/// match the exact function-result shape the real harness persists.
fn normalize_function_response(response: &Value) -> (Value, bool) {
    let is_error = response
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Value::String(text) = response {
        return (json!([{ "type": "text", "text": text }]), is_error);
    }
    if let Some(content) = response.get("content") {
        if let Ok(blocks) = serde_json::from_value::<Vec<ContentBlock>>(content.clone()) {
            if !blocks.is_empty() {
                return (
                    serde_json::to_value(blocks).expect("content blocks serialize"),
                    is_error,
                );
            }
        }
    }
    let rendered =
        serde_json::to_string(response).unwrap_or_else(|_| "<unserializable>".to_string());
    (json!([{ "type": "text", "text": rendered }]), is_error)
}
