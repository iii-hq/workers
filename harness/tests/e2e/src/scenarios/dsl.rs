//! Small typed vocabulary for authoring checked-in scenarios.
//!
//! Builders remove wire-format repetition but compile directly to the runtime
//! types. Request differences and function history remain explicit at each
//! scenario call site.

use serde_json::{json, Value};

use super::{ScenarioDriver, VerifyFn};
use crate::fixtures::ScenarioFixture;
use crate::types::frames::{
    AssistantMessage, AssistantMessageEvent, AssistantRoleTag, ContentBlock, RouterChatResponse,
    StopReason, Usage,
};
use crate::types::recorder::{
    LifecycleFunctionId, LifecycleTriggerType, RecorderConfigV1, RecorderLifecycleV1,
    RecorderTargetV1,
};
use crate::types::scenario::{
    CompiledFunctionExposureV1, CompiledFunctionPolicyV1, CompiledScenarioV1,
    CompiledSendOptionsV1, CompiledSendV1, DeadlinesV1,
};
use crate::types::script::{
    GenerationMatchV1, JsonMatcherV1, JsonNormalizerV1, ModelFixtureV1, NormalizerOperation,
    RouterScriptV1, SchemaVersion1, ScriptedGenerationV1,
};

const DEFAULT_SYSTEM_PROMPT: &str = include_str!("../../../../prompts/default.txt");
const PROVIDER: &str = "scripted";

pub(super) struct Model;

impl Model {
    pub(super) fn scripted(id: &str) -> ModelFixtureV1 {
        ModelFixtureV1 {
            id: id.to_string(),
            provider: PROVIDER.to_string(),
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
}

pub(super) struct Scenario {
    id: String,
    slug: String,
    description: String,
    driver: ScenarioDriver,
    model: ModelFixtureV1,
    send: Option<Send>,
    recorder: Option<RecorderConfigV1>,
    generations: Vec<Generation>,
    verify: Option<VerifyFn>,
}

impl Scenario {
    pub(super) fn new(
        id: &str,
        slug: &str,
        description: &str,
        driver: ScenarioDriver,
        model: ModelFixtureV1,
    ) -> Self {
        Self {
            id: id.to_string(),
            slug: slug.to_string(),
            description: description.to_string(),
            driver,
            model,
            send: None,
            recorder: None,
            generations: Vec::new(),
            verify: None,
        }
    }

    pub(super) fn send(mut self, send: Send) -> Self {
        self.send = Some(send);
        self
    }

    pub(super) fn recorder(mut self, recorder: RecorderConfigV1) -> Self {
        self.recorder = Some(recorder);
        self
    }

    pub(super) fn generation(mut self, generation: Generation) -> Self {
        self.generations.push(generation);
        self
    }

    pub(super) fn verify(mut self, verify: VerifyFn) -> Self {
        self.verify = Some(verify);
        self
    }

    pub(super) fn build(self) -> ScenarioFixture {
        let send = self.send.expect("scenario send is required");
        let allowed_functions = send.allowed_functions.clone();
        let compiled_send = send.compile(&self.id, &self.model);
        let recorder = self.recorder.expect("scenario recorder is required");
        let generations = self
            .generations
            .into_iter()
            .map(|generation| generation.compile(&self.model))
            .collect();

        ScenarioFixture {
            slug: self.slug,
            driver: self.driver,
            scenario: CompiledScenarioV1 {
                schema_version: SchemaVersion1::V1,
                id: self.id.clone(),
                description: self.description,
                send: compiled_send,
                recorder,
                deadlines: DeadlinesV1::default(),
            },
            script: RouterScriptV1 {
                schema_version: SchemaVersion1::V1,
                scenario_id: self.id,
                model: self.model,
                generations,
            },
            system_prompt_template: system_prompt(&allowed_functions),
            verify: self.verify.expect("scenario verification is required"),
        }
    }
}

pub(super) struct Send {
    message: String,
    idempotency_key: Option<String>,
    allowed_functions: Vec<String>,
}

impl Send {
    pub(super) fn message(message: &str) -> Self {
        Self {
            message: message.to_string(),
            idempotency_key: None,
            allowed_functions: Vec::new(),
        }
    }

    pub(super) fn idempotency_key(mut self, key: &str) -> Self {
        self.idempotency_key = Some(key.to_string());
        self
    }

    pub(super) fn without_functions(self) -> Self {
        self
    }

    pub(super) fn allow_function(mut self, function: &RecorderFunction) -> Self {
        if !self
            .allowed_functions
            .contains(&function.target.function_id)
        {
            self.allowed_functions
                .push(function.target.function_id.clone());
        }
        self
    }

    fn compile(self, scenario_id: &str, model: &ModelFixtureV1) -> CompiledSendV1 {
        CompiledSendV1 {
            session_id: "{{session_id}}".to_string(),
            message: self.message,
            model: model.id.clone(),
            provider: model.provider.clone(),
            idempotency_key: self
                .idempotency_key
                .unwrap_or_else(|| format!("{{{{run_id}}}}:{}", scenario_id.to_ascii_lowercase())),
            options: CompiledSendOptionsV1 {
                functions: CompiledFunctionPolicyV1 {
                    allow: self.allowed_functions,
                    deny: Vec::new(),
                    expose: CompiledFunctionExposureV1::Native,
                },
            },
        }
    }
}

pub(super) struct Recorder;

impl Recorder {
    pub(super) fn lifecycle_only() -> RecorderConfigV1 {
        recorder_config(RecorderTargetV1 {
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

    pub(super) fn function(function: RecorderFunction) -> RecorderConfigV1 {
        recorder_config(function.target)
    }
}

#[derive(Clone)]
pub(super) struct RecorderFunction {
    target: RecorderTargetV1,
}

impl RecorderFunction {
    pub(super) fn new(function_id: &str, description: &str) -> Self {
        Self {
            target: RecorderTargetV1 {
                function_id: function_id.to_string(),
                description: description.to_string(),
                request_schema: serde_json::Map::new(),
                response: Value::Null,
            },
        }
    }

    pub(super) fn request_schema(mut self, schema: Value) -> Self {
        self.target.request_schema = schema
            .as_object()
            .unwrap_or_else(|| panic!("recorder request schema must be an object: {schema}"))
            .clone();
        self
    }

    pub(super) fn returns_text(mut self, text: &str) -> Self {
        self.target.response = json!({
            "content": [{ "type": "text", "text": text }],
            "is_error": false
        });
        self
    }

    pub(super) fn id(&self) -> &str {
        &self.target.function_id
    }

    pub(super) fn tool(&self) -> Value {
        json!({
            "name": self.target.function_id,
            "description": self.target.description,
            "parameters": self.target.request_schema,
            "execution_mode": "sequential"
        })
    }
}

pub(super) struct Request {
    turn_request: bool,
    system_prompt: Option<JsonMatcherV1>,
    messages: Option<JsonMatcherV1>,
    tools: Option<JsonMatcherV1>,
}

impl Request {
    pub(super) fn new() -> Self {
        Self {
            turn_request: false,
            system_prompt: None,
            messages: None,
            tools: None,
        }
    }

    pub(super) fn turn_request(mut self) -> Self {
        self.turn_request = true;
        self
    }

    pub(super) fn system_prompt_sha256(mut self, expected: &str) -> Self {
        self.system_prompt = Some(JsonMatcherV1::Sha256 {
            expected: expected.to_string(),
        });
        self
    }

    pub(super) fn system_prompt_regex(mut self, pattern: &str) -> Self {
        self.system_prompt = Some(JsonMatcherV1::Regex {
            pattern: pattern.to_string(),
        });
        self
    }

    pub(super) fn messages_exact(mut self, messages: impl IntoIterator<Item = Value>) -> Self {
        let messages: Vec<Value> = messages.into_iter().collect();
        let normalize = (0..messages.len())
            .map(|index| JsonNormalizerV1 {
                pointer: format!("/{index}/timestamp"),
                operation: NormalizerOperation::Delete,
            })
            .collect();
        self.messages = Some(JsonMatcherV1::Exact {
            expected: Value::Array(messages),
            normalize: Some(normalize),
        });
        self
    }

    pub(super) fn without_tools(mut self) -> Self {
        self.tools = Some(exact(json!([])));
        self
    }

    pub(super) fn tools_exact(mut self, tools: impl IntoIterator<Item = Value>) -> Self {
        self.tools = Some(exact(Value::Array(tools.into_iter().collect())));
        self
    }

    pub(super) fn tools_subset(mut self, tools: impl IntoIterator<Item = Value>) -> Self {
        self.tools = Some(JsonMatcherV1::Subset {
            expected: Value::Array(tools.into_iter().collect()),
            normalize: None,
        });
        self
    }

    fn compile(self, model: &ModelFixtureV1, ordinal: u64) -> GenerationMatchV1 {
        assert!(self.turn_request, "request id matcher is required");
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
            system_prompt: self
                .system_prompt
                .expect("system prompt matcher is required"),
            messages: self.messages.expect("message matcher is required"),
            tools: self.tools.expect("tool matcher is required"),
            response_format: JsonMatcherV1::Absent,
            thinking_level: JsonMatcherV1::Absent,
            max_output_tokens: JsonMatcherV1::Absent,
            provider_options: JsonMatcherV1::Absent,
            metadata: JsonMatcherV1::Absent,
        }
    }
}

pub(super) struct Generation {
    ordinal: u64,
    request: Option<Request>,
    response: Option<Response>,
}

impl Generation {
    pub(super) fn new(ordinal: u64) -> Self {
        Self {
            ordinal,
            request: None,
            response: None,
        }
    }

    pub(super) fn expect(mut self, request: Request) -> Self {
        self.request = Some(request);
        self
    }

    pub(super) fn respond(mut self, response: Response) -> Self {
        self.response = Some(response);
        self
    }

    fn compile(self, model: &ModelFixtureV1) -> ScriptedGenerationV1 {
        let (frames, response) = self
            .response
            .expect("generation response is required")
            .compile(model, self.ordinal);
        ScriptedGenerationV1 {
            ordinal: self.ordinal,
            match_: self
                .request
                .expect("generation request is required")
                .compile(model, self.ordinal),
            frames,
            response,
        }
    }
}

pub(super) struct Response {
    kind: ResponseKind,
    usage: Usage,
}

enum ResponseKind {
    StreamedText {
        text: String,
        chunks: Vec<String>,
    },
    Text(String),
    FunctionCall {
        call_id: String,
        function_id: String,
        arguments: Value,
    },
}

impl Response {
    pub(super) fn streamed_text<I, S>(
        text: &str,
        chunks: I,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            kind: ResponseKind::StreamedText {
                text: text.to_string(),
                chunks: chunks.into_iter().map(Into::into).collect(),
            },
            usage: usage(input_tokens, output_tokens),
        }
    }

    pub(super) fn text(text: &str, input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            kind: ResponseKind::Text(text.to_string()),
            usage: usage(input_tokens, output_tokens),
        }
    }

    pub(super) fn function_call(
        call_id: &str,
        function: &RecorderFunction,
        arguments: Value,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Self {
        Self {
            kind: ResponseKind::FunctionCall {
                call_id: call_id.to_string(),
                function_id: function.id().to_string(),
                arguments,
            },
            usage: usage(input_tokens, output_tokens),
        }
    }

    fn compile(
        self,
        model: &ModelFixtureV1,
        ordinal: u64,
    ) -> (Vec<AssistantMessageEvent>, RouterChatResponse) {
        let usage = self.usage;
        let timestamp = i64::try_from(ordinal).expect("generation ordinal fits i64");
        let (frames, stop_reason) = match self.kind {
            ResponseKind::StreamedText { text, chunks } => (
                streamed_text_frames(&text, &chunks, &usage, model, timestamp),
                StopReason::End,
            ),
            ResponseKind::Text(text) => (
                vec![AssistantMessageEvent::Done {
                    message: assistant_message(
                        vec![ContentBlock::Text { text }],
                        StopReason::End,
                        Some(usage.clone()),
                        model,
                        timestamp,
                    ),
                }],
                StopReason::End,
            ),
            ResponseKind::FunctionCall {
                call_id,
                function_id,
                arguments,
            } => (
                vec![AssistantMessageEvent::Done {
                    message: assistant_message(
                        vec![ContentBlock::FunctionCall {
                            id: call_id,
                            function_id,
                            arguments,
                        }],
                        StopReason::FunctionCall,
                        Some(usage.clone()),
                        model,
                        timestamp,
                    ),
                }],
                StopReason::FunctionCall,
            ),
        };
        let response = RouterChatResponse {
            ok: true,
            provider: model.provider.clone(),
            model: model.id.clone(),
            stop_reason: Some(stop_reason),
            usage: Some(usage),
            error: None,
        };
        (frames, response)
    }
}

pub(super) struct Message;

impl Message {
    pub(super) fn user(text: &str) -> Value {
        json!({
            "role": "user",
            "content": [{ "type": "text", "text": text }]
        })
    }

    pub(super) fn function_call(
        call_id: &str,
        function: &RecorderFunction,
        arguments: Value,
        model: &ModelFixtureV1,
    ) -> Value {
        json!({
            "role": "assistant",
            "content": [{
                "type": "function_call",
                "id": call_id,
                "function_id": function.id(),
                "arguments": arguments
            }],
            "stop_reason": "end",
            "model": model.id,
            "provider": model.provider
        })
    }

    pub(super) fn function_result(call_id: &str, function: &RecorderFunction, text: &str) -> Value {
        json!({
            "role": "function_result",
            "function_call_id": call_id,
            "function_id": function.id(),
            "content": [{ "type": "text", "text": text }],
            "details": function.target.response,
            "is_error": false
        })
    }
}

pub(super) struct Tool;

impl Tool {
    pub(super) fn named(name: &str) -> Value {
        json!({ "name": name })
    }
}

fn recorder_config(target: RecorderTargetV1) -> RecorderConfigV1 {
    RecorderConfigV1 {
        target,
        lifecycle: RecorderLifecycleV1 {
            trigger_type: LifecycleTriggerType::TurnCompleted,
            function_id: LifecycleFunctionId::Lifecycle,
        },
    }
}

fn usage(input: u64, output: u64) -> Usage {
    Usage {
        input: Some(input),
        output: Some(output),
        ..Default::default()
    }
}

fn exact(expected: Value) -> JsonMatcherV1 {
    JsonMatcherV1::Exact {
        expected,
        normalize: None,
    }
}

fn assistant_message(
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

fn streamed_text_frames(
    text: &str,
    chunks: &[String],
    usage: &Usage,
    model: &ModelFixtureV1,
    timestamp: i64,
) -> Vec<AssistantMessageEvent> {
    let final_message = assistant_message(
        vec![ContentBlock::Text {
            text: text.to_string(),
        }],
        StopReason::End,
        Some(usage.clone()),
        model,
        timestamp,
    );
    if chunks.is_empty() {
        return vec![AssistantMessageEvent::Done {
            message: final_message,
        }];
    }
    let mut frames = vec![
        AssistantMessageEvent::Start {
            partial: assistant_message(Vec::new(), StopReason::End, None, model, timestamp),
        },
        AssistantMessageEvent::TextStart {
            partial: assistant_message(
                vec![ContentBlock::Text {
                    text: String::new(),
                }],
                StopReason::End,
                None,
                model,
                timestamp,
            ),
        },
    ];
    frames.extend(
        chunks
            .iter()
            .cloned()
            .map(|delta| AssistantMessageEvent::TextDelta {
                partial: None,
                delta,
            }),
    );
    frames.extend([
        AssistantMessageEvent::TextEnd {
            partial: assistant_message(
                vec![ContentBlock::Text {
                    text: text.to_string(),
                }],
                StopReason::End,
                None,
                model,
                timestamp,
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
        AssistantMessageEvent::Done {
            message: final_message,
        },
    ]);
    frames
}

fn system_prompt(allowed_functions: &[String]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_response_has_one_terminal_frame() {
        let model = Model::scripted("fixture-model");
        let (frames, response) =
            Response::streamed_text("complete", ["com", "plete"], 2, 1).compile(&model, 1);
        assert_eq!(frames.iter().filter(|frame| frame.is_terminal()).count(), 1);
        assert!(frames.last().unwrap().is_terminal());
        assert_eq!(response.stop_reason, Some(StopReason::End));
    }

    #[test]
    fn recorder_function_uses_one_contract_for_tool_and_target() {
        let function = RecorderFunction::new("{{run_id}}::record", "Record value")
            .request_schema(json!({ "type": "object" }))
            .returns_text("recorded");
        let tool = function.tool();
        let recorder = Recorder::function(function.clone());
        assert_eq!(tool["name"], recorder.target.function_id);
        assert_eq!(
            tool["parameters"],
            Value::Object(recorder.target.request_schema.clone())
        );
        assert_eq!(function.target.response, recorder.target.response);
    }
}
