//! Shared wire constructors used by the checked-in scenarios.

use serde_json::{json, Value};

use crate::types::frames::{
    AssistantMessage, AssistantMessageEvent, AssistantRoleTag, ContentBlock, RouterChatResponse,
    StopReason, Usage,
};
use crate::types::recorder::{
    LifecycleFunctionId, LifecycleTriggerType, RecorderConfigV1, RecorderLifecycleV1,
    RecorderTargetV1,
};
use crate::types::scenario::{
    CompiledFunctionExposureV1, CompiledFunctionPolicyV1, CompiledSendOptionsV1, CompiledSendV1,
};
use crate::types::script::{
    GenerationMatchV1, JsonMatcherV1, JsonNormalizerV1, ModelFixtureV1, NormalizerOperation,
};

const DEFAULT_SYSTEM_PROMPT: &str = include_str!("../../../../prompts/default.txt");
pub(super) const MODEL_ID: &str = "fixture-model";
pub(super) const PROVIDER_ID: &str = "scripted";

#[derive(Debug, Clone, Copy)]
pub(super) enum RequestProfile {
    Direct,
    Console,
}

pub(super) fn model() -> ModelFixtureV1 {
    ModelFixtureV1 {
        id: MODEL_ID.to_string(),
        provider: PROVIDER_ID.to_string(),
        context_window: 32_768,
        max_output_tokens: 4_096,
        supports_thinking: Some(false),
        supports_xhigh: None,
        supports_tools: Some(true),
        supports_vision: Some(false),
        supports_cache: Some(false),
        supports_structured_output: Some(true),
    }
}

pub(super) fn send(
    scenario_id: &str,
    message: &str,
    model: &ModelFixtureV1,
    allowed_functions: &[String],
) -> CompiledSendV1 {
    CompiledSendV1 {
        session_id: "{{session_id}}".to_string(),
        message: message.to_string(),
        model: model.id.clone(),
        provider: model.provider.clone(),
        idempotency_key: format!("{{{{run_id}}}}:{}", scenario_id.to_ascii_lowercase()),
        options: CompiledSendOptionsV1 {
            functions: CompiledFunctionPolicyV1 {
                allow: allowed_functions.to_vec(),
                deny: Vec::new(),
                expose: CompiledFunctionExposureV1::Native,
            },
        },
    }
}

pub(super) fn request_match(
    ordinal: u64,
    model: &ModelFixtureV1,
    messages: &[Value],
    tools: &Value,
    profile: RequestProfile,
) -> GenerationMatchV1 {
    let normalizers = (0..messages.len())
        .map(|index| JsonNormalizerV1 {
            pointer: format!("/{index}/timestamp"),
            operation: NormalizerOperation::Delete,
        })
        .collect();
    let (system_prompt, tools) = match profile {
        RequestProfile::Direct => (
            JsonMatcherV1::Sha256 {
                expected: "{{system_prompt_sha256}}".to_string(),
            },
            exact(tools.clone()),
        ),
        RequestProfile::Console => (
            JsonMatcherV1::Regex {
                pattern: "agent_trigger".to_string(),
            },
            JsonMatcherV1::Subset {
                expected: json!([{ "name": "agent_trigger" }]),
                normalize: None,
            },
        ),
    };
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
        model: exact(json!(model.id)),
        provider: exact(json!(model.provider)),
        system_prompt,
        messages: JsonMatcherV1::Exact {
            expected: Value::Array(messages.to_vec()),
            normalize: Some(normalizers),
        },
        tools,
        response_format: JsonMatcherV1::Absent,
        thinking_level: JsonMatcherV1::Absent,
        max_output_tokens: JsonMatcherV1::Absent,
        provider_options: JsonMatcherV1::Absent,
        metadata: JsonMatcherV1::Absent,
    }
}

fn exact(expected: Value) -> JsonMatcherV1 {
    JsonMatcherV1::Exact {
        expected,
        normalize: None,
    }
}

pub(super) fn user_message(message: &str) -> Value {
    json!({
        "role": "user",
        "content": [{ "type": "text", "text": message }]
    })
}

pub(super) fn assistant_message(
    content: Vec<ContentBlock>,
    stop_reason: StopReason,
    usage: Option<Usage>,
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

pub(super) fn streamed_text_frames(
    text: &str,
    chunks: &[&str],
    usage: &Usage,
    model: &ModelFixtureV1,
) -> Vec<AssistantMessageEvent> {
    let message = assistant_message(
        vec![ContentBlock::Text {
            text: text.to_string(),
        }],
        StopReason::End,
        Some(usage.clone()),
        model,
        1,
    );
    if chunks.is_empty() {
        return vec![AssistantMessageEvent::Done { message }];
    }

    let mut frames = vec![
        AssistantMessageEvent::Start {
            partial: assistant_message(Vec::new(), StopReason::End, None, model, 1),
        },
        AssistantMessageEvent::TextStart {
            partial: assistant_message(
                vec![ContentBlock::Text {
                    text: String::new(),
                }],
                StopReason::End,
                None,
                model,
                1,
            ),
        },
    ];
    frames.extend(chunks.iter().map(|chunk| AssistantMessageEvent::TextDelta {
        partial: None,
        delta: (*chunk).to_string(),
    }));
    frames.extend([
        AssistantMessageEvent::TextEnd {
            partial: assistant_message(
                vec![ContentBlock::Text {
                    text: text.to_string(),
                }],
                StopReason::End,
                None,
                model,
                1,
            ),
        },
        AssistantMessageEvent::Usage {
            usage: usage.clone(),
        },
        AssistantMessageEvent::Stop {
            stop_reason: StopReason::End,
            error_message: None,
            error_kind: None,
        },
        AssistantMessageEvent::Done { message },
    ]);
    frames
}

pub(super) fn response(
    stop_reason: StopReason,
    usage: Usage,
    model: &ModelFixtureV1,
) -> RouterChatResponse {
    RouterChatResponse {
        ok: true,
        provider: model.provider.clone(),
        model: model.id.clone(),
        stop_reason: Some(stop_reason),
        usage: Some(usage),
        error: None,
    }
}

pub(super) fn usage(input: u64, output: u64) -> Usage {
    Usage {
        input: Some(input),
        output: Some(output),
        ..Default::default()
    }
}

pub(super) fn recorder(target: RecorderTargetV1) -> RecorderConfigV1 {
    RecorderConfigV1 {
        target,
        lifecycle: RecorderLifecycleV1 {
            trigger_type: LifecycleTriggerType::TurnCompleted,
            function_id: LifecycleFunctionId::Lifecycle,
        },
    }
}

pub(super) fn synthetic_recorder() -> RecorderConfigV1 {
    recorder(RecorderTargetV1 {
        function_id: "{{run_id}}::unused".to_string(),
        description: "Synthetic integration target; must never be called.".to_string(),
        request_schema: json!({
            "type": "object",
            "additionalProperties": false
        })
        .as_object()
        .expect("object")
        .clone(),
        response: json!({
            "content": [{ "type": "text", "text": "unused" }],
            "is_error": false
        }),
    })
}

pub(super) fn recorder_target(function_id: &str) -> RecorderTargetV1 {
    RecorderTargetV1 {
        function_id: function_id.to_string(),
        description: "Record one integration fixture value.".to_string(),
        request_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "value": { "type": "string" } },
            "required": ["value"]
        })
        .as_object()
        .expect("object")
        .clone(),
        response: json!({
            "content": [{ "type": "text", "text": "recorded" }],
            "is_error": false
        }),
    }
}

pub(super) fn system_prompt(allowed_functions: &[String]) -> String {
    let base = DEFAULT_SYSTEM_PROMPT
        .strip_suffix('\n')
        .unwrap_or(DEFAULT_SYSTEM_PROMPT);
    let policy = if allowed_functions.is_empty() {
        "Function dispatch is entirely disabled this turn — do not call any function.".to_string()
    } else {
        format!(
            "Your dispatch policy allows ONLY these functions: {}. This narrowed-policy \
             instruction OVERRIDES the general discovery requirement for this turn: call the \
             listed target ids directly when the task already supplies their arguments. Anything \
             else — including discovery (engine::functions::list / ::info) unless listed above — \
             is denied. Do not probe: if the task genuinely needs an unlisted function or an \
             unknown contract, report that blocker and finish.",
            allowed_functions.join(", ")
        )
    };
    format!("{base}\n\nYour session id is {{{{session_id}}}}.\n{policy}")
}
