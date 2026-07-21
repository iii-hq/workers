//! Typed builders for authored scenarios.
//!
//! Builders produce data only: every method fills a field on the authored
//! structs in [`crate::types::scenario`] and returns the value for further
//! chaining. A builder that derives scenario content from control flow is
//! rejected in review, under the same rule that forbids a second
//! orchestration language in the authored layer.
//!
//! Defaults mirror the deterministic compiler defaults: a text reply without
//! chunks emits one terminal `done` frame, faults interrupt the first target
//! call after 1500 ms, and releases target `call-1`.

use serde_json::{json, Value};

use crate::types::frames::Usage;
use crate::types::scenario::{
    AuthoredScenarioV1, ExpectationsV1, FaultKind, FaultV1, FunctionResultExpectationV1,
    GenerationMatchOverridesV1, ReleaseActionV1, ReleaseV1, RouterReplyV1, ScenarioFunctionV1,
    ScenarioGenerationV1, ScenarioRouterV1, ScenarioSendV1, TargetCallsExpectationV1,
    TriggerBindingSpecV1, TriggerKindV1,
};
use crate::types::script::{JsonMatcherV1, SchemaVersion1};

/// Authoring name for the scenario root. Scenario modules return this type.
pub type AuthoredScenario = AuthoredScenarioV1;

impl AuthoredScenarioV1 {
    /// A new scenario with an empty send. Chain [`Self::send`] before
    /// registering; the registry tests reject an empty message.
    pub fn new(id: &str, description: &str) -> Self {
        Self {
            schema_version: SchemaVersion1::V1,
            id: id.to_string(),
            description: description.to_string(),
            quarantine: false,
            send: Send::message(""),
            functions: Default::default(),
            router: ScenarioRouterV1 {
                model: None,
                generations: Vec::new(),
            },
            bindings: Vec::new(),
            release: None,
            fault: None,
            timeouts: Default::default(),
            expect: Default::default(),
        }
    }

    /// Exclude this scenario from ordinary `all` runs; `validate` and
    /// explicit selection still include it.
    pub fn quarantine(mut self) -> Self {
        self.quarantine = true;
        self
    }

    pub fn send(mut self, send: ScenarioSendV1) -> Self {
        self.send = send;
        self
    }

    /// Register a controlled function under its alias. The compiler expands
    /// aliases to `{{run_id}}::<alias>`.
    pub fn function(mut self, alias: &str, function: ScenarioFunctionV1) -> Self {
        self.functions.insert(alias.to_string(), function);
        self
    }

    pub fn generation(mut self, generation: impl Into<ScenarioGenerationV1>) -> Self {
        self.router.generations.push(generation.into());
        self
    }

    pub fn binding(mut self, binding: TriggerBindingSpecV1) -> Self {
        self.bindings.push(binding);
        self
    }

    /// Release the deterministic first call (`call-1`) once it is held.
    pub fn release(mut self, action: ReleaseActionV1) -> Self {
        self.release = Some(ReleaseV1 {
            function_call_id: "call-1".to_string(),
            action,
        });
        self
    }

    pub fn fault(mut self, fault: FaultV1) -> Self {
        self.fault = Some(fault);
        self
    }

    pub fn readiness_timeout_ms(mut self, readiness_ms: u64) -> Self {
        self.timeouts.readiness_ms = readiness_ms;
        self
    }

    pub fn scenario_timeout_ms(mut self, scenario_ms: u64) -> Self {
        self.timeouts.scenario_ms = scenario_ms;
        self
    }

    pub fn teardown_timeout_ms(mut self, teardown_ms: u64) -> Self {
        self.timeouts.teardown_ms = teardown_ms;
        self
    }

    pub fn expect(mut self, expect: ExpectationsV1) -> Self {
        self.expect = expect;
        self
    }
}

/// Entry point for [`ScenarioSendV1`]. Shadows the marker trait name inside
/// scenario modules on purpose; those modules contain data only.
pub struct Send;

impl Send {
    pub fn message(message: &str) -> ScenarioSendV1 {
        ScenarioSendV1 {
            message: message.to_string(),
            allow: None,
            idempotency_key: None,
        }
    }
}

impl ScenarioSendV1 {
    /// Allowed function aliases. Omitting the call keeps every exposed
    /// function; `allow([])` disables function dispatch.
    pub fn allow<const N: usize>(mut self, aliases: [&str; N]) -> Self {
        self.allow = Some(aliases.iter().map(|alias| alias.to_string()).collect());
        self
    }

    pub fn idempotency_key(mut self, key: &str) -> Self {
        self.idempotency_key = Some(key.to_string());
        self
    }
}

/// Authoring name for [`ScenarioFunctionV1`].
pub type Function = ScenarioFunctionV1;

impl ScenarioFunctionV1 {
    /// A controlled function exposed to the model. `request_schema` must be
    /// a JSON object.
    pub fn new(description: &str, request_schema: Value, response: Value) -> Self {
        Self {
            description: description.to_string(),
            request_schema: request_schema
                .as_object()
                .cloned()
                .expect("request_schema must be a JSON object"),
            response,
            expose: true,
        }
    }

    /// The canonical recorder fixture: one required string `value`, returning
    /// a durable `recorded` text result. It is the most common controlled
    /// function; chain [`Self::hidden`] for a hook-only variant.
    pub fn recorder() -> Self {
        Self::new(
            "Record one integration fixture value.",
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "value": { "type": "string" } },
                "required": ["value"]
            }),
            json!({
                "content": [{ "type": "text", "text": "recorded" }],
                "is_error": false
            }),
        )
    }

    /// Hook-only controlled functions are never exposed to the model.
    pub fn hidden(mut self) -> Self {
        self.expose = false;
        self
    }
}

/// Entry point for typed router replies.
pub struct Reply;

impl Reply {
    pub fn text(text: &str) -> TextReply {
        TextReply {
            text: text.to_string(),
            chunks: Vec::new(),
            usage: None,
        }
    }

    /// A function call against a registered alias, closed as
    /// `call-<function-call ordinal>` by the compiler.
    pub fn function_call(function: &str, arguments: Value) -> FunctionCallReply {
        FunctionCallReply {
            function: function.to_string(),
            arguments,
            usage: None,
        }
    }
}

pub struct TextReply {
    text: String,
    chunks: Vec<String>,
    usage: Option<Usage>,
}

impl TextReply {
    /// Non-empty chunks produce the complete streaming frame sequence;
    /// without chunks the reply is one terminal `done` frame.
    pub fn chunks<const N: usize>(mut self, chunks: [&str; N]) -> Self {
        self.chunks = chunks.iter().map(|chunk| chunk.to_string()).collect();
        self
    }

    pub fn usage(mut self, input: u64, output: u64) -> Self {
        self.usage = Some(input_output_usage(input, output));
        self
    }

    pub fn match_overrides(self, overrides: GenerationMatchOverridesV1) -> ScenarioGenerationV1 {
        with_overrides(self.into(), overrides)
    }

    /// Grade this reply against the durable outcome only, at a recovery
    /// boundary (fault restart or hook release) where the engine may rebuild
    /// the request differently. Shorthand for [`Self::match_overrides`] with
    /// the recovery policy.
    pub fn recovery_boundary(self) -> ScenarioGenerationV1 {
        with_overrides(self.into(), recovery_overrides())
    }
}

impl From<TextReply> for ScenarioGenerationV1 {
    fn from(reply: TextReply) -> Self {
        plain_generation(RouterReplyV1::Text {
            text: reply.text,
            chunks: reply.chunks,
            usage: reply.usage,
        })
    }
}

pub struct FunctionCallReply {
    function: String,
    arguments: Value,
    usage: Option<Usage>,
}

impl FunctionCallReply {
    pub fn usage(mut self, input: u64, output: u64) -> Self {
        self.usage = Some(input_output_usage(input, output));
        self
    }

    pub fn match_overrides(self, overrides: GenerationMatchOverridesV1) -> ScenarioGenerationV1 {
        with_overrides(self.into(), overrides)
    }

    /// Grade this reply against the durable outcome only, at a recovery
    /// boundary (fault restart or hook release) where the engine may rebuild
    /// the request differently. Shorthand for [`Self::match_overrides`] with
    /// the recovery policy.
    pub fn recovery_boundary(self) -> ScenarioGenerationV1 {
        with_overrides(self.into(), recovery_overrides())
    }
}

impl From<FunctionCallReply> for ScenarioGenerationV1 {
    fn from(reply: FunctionCallReply) -> Self {
        plain_generation(RouterReplyV1::FunctionCall {
            id: None,
            function: reply.function,
            arguments: reply.arguments,
            usage: reply.usage,
        })
    }
}

fn plain_generation(reply: RouterReplyV1) -> ScenarioGenerationV1 {
    ScenarioGenerationV1 {
        reply,
        match_overrides: Default::default(),
    }
}

fn with_overrides(
    mut generation: ScenarioGenerationV1,
    overrides: GenerationMatchOverridesV1,
) -> ScenarioGenerationV1 {
    generation.match_overrides = overrides;
    generation
}

/// The loose match a recovery-boundary reply needs. After a fault restart or a
/// hook release the engine may legitimately reconstruct the request
/// differently, so grade the durable shape and leave the reconstructed request
/// id and body free.
fn recovery_overrides() -> GenerationMatchOverridesV1 {
    GenerationMatchOverridesV1 {
        request_id: Some(regex("^t_[0-9a-f]{32}:[0-9]+$")),
        system_prompt: Some(present()),
        messages: Some(present()),
        tools: Some(present()),
        ..Default::default()
    }
}

fn input_output_usage(input: u64, output: u64) -> Usage {
    Usage {
        input: Some(input),
        output: Some(output),
        cache_read: None,
        cache_write: None,
        reasoning: None,
        cost_usd: None,
    }
}

/// Entry point for [`TriggerBindingSpecV1`].
pub struct Binding;

impl Binding {
    pub fn hook_pre_trigger<const N: usize>(
        function: &str,
        functions: [&str; N],
        priority: i64,
    ) -> TriggerBindingSpecV1 {
        TriggerBindingSpecV1 {
            trigger: TriggerKindV1::HookPreTrigger,
            function: function.to_string(),
            functions: functions.iter().map(|alias| alias.to_string()).collect(),
            priority,
        }
    }
}

/// Entry point for [`FaultV1`], mirroring the deterministic defaults the
/// YAML layer used to apply.
pub struct Fault;

impl Fault {
    pub fn engine_sigkill() -> FaultV1 {
        FaultV1 {
            kind: FaultKind::EngineSigkill,
            function: None,
            after_target_calls: 1,
            restart_delay_ms: 1_500,
        }
    }
}

impl FaultV1 {
    pub fn function(mut self, alias: &str) -> Self {
        self.function = Some(alias.to_string());
        self
    }

    pub fn after_target_calls(mut self, calls: u64) -> Self {
        self.after_target_calls = calls;
        self
    }

    pub fn restart_delay_ms(mut self, delay_ms: u64) -> Self {
        self.restart_delay_ms = delay_ms;
        self
    }
}

/// Entry point for [`ReleaseActionV1`], the action applied to the held
/// deterministic first call by [`AuthoredScenarioV1::release`].
pub struct Release;

impl Release {
    /// Release the held call for execution against its target function.
    pub fn execute() -> ReleaseActionV1 {
        ReleaseActionV1::Execute
    }

    /// Release the held call by delivering its recorded result without
    /// re-execution.
    pub fn deliver() -> ReleaseActionV1 {
        ReleaseActionV1::Deliver
    }
}

/// Authoring name for [`ExpectationsV1`].
pub type Expect = ExpectationsV1;

impl ExpectationsV1 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn message_counts(mut self, user: u64, assistant: u64, function_result: u64) -> Self {
        self.message_counts = Some(crate::types::scenario::MessageCountsExpectationV1 {
            user,
            assistant,
            function_result,
        });
        self
    }

    pub fn assistant_text(mut self, text: &str) -> Self {
        self.assistant_text = Some(text.to_string());
        self
    }

    pub fn function_result(mut self, result: FunctionResultExpectationV1) -> Self {
        self.function_results.push(result);
        self
    }

    pub fn calls_closed(mut self) -> Self {
        self.calls_closed = true;
        self
    }

    pub fn call(mut self, call: TargetCallsExpectationV1) -> Self {
        self.calls.push(call);
        self
    }
}

/// Entry point for [`FunctionResultExpectationV1`].
pub struct FunctionResult;

impl FunctionResult {
    /// A durable function result closing the given call id, with no further
    /// constraints until chained.
    pub fn closing(function_call_id: &str) -> FunctionResultExpectationV1 {
        FunctionResultExpectationV1 {
            function_call_id: function_call_id.to_string(),
            function: None,
            content: None,
            is_error: None,
        }
    }
}

impl FunctionResultExpectationV1 {
    pub fn function(mut self, alias: &str) -> Self {
        self.function = Some(alias.to_string());
        self
    }

    pub fn content(mut self, content: Vec<Value>) -> Self {
        self.content = Some(content);
        self
    }

    pub fn is_error(mut self, is_error: bool) -> Self {
        self.is_error = Some(is_error);
        self
    }
}

/// Entry point for [`TargetCallsExpectationV1`].
pub struct TargetCall;

impl TargetCall {
    pub fn counted(function: &str, count: u64) -> TargetCallsExpectationV1 {
        TargetCallsExpectationV1 {
            function: function.to_string(),
            count,
            payload: None,
            payload_subset: None,
        }
    }
}

impl TargetCallsExpectationV1 {
    pub fn payload(mut self, payload: Value) -> Self {
        self.payload = Some(payload);
        self
    }

    pub fn payload_subset(mut self, payload_subset: Value) -> Self {
        self.payload_subset = Some(payload_subset);
        self
    }
}

pub fn regex(pattern: &str) -> JsonMatcherV1 {
    JsonMatcherV1::Regex {
        pattern: pattern.to_string(),
    }
}

pub fn present() -> JsonMatcherV1 {
    JsonMatcherV1::Present
}

pub fn subset(expected: Value) -> JsonMatcherV1 {
    JsonMatcherV1::Subset {
        expected,
        normalize: None,
    }
}
