//! `RouterScriptV1` — the strict scripted-router fixture. Loaded, expanded, and validated before the
//! stack boots; any defect here is a `runner_error`, never a subject failure.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::frames::{AssistantMessageEvent, RouterChatResponse};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouterScriptV1 {
    pub schema_version: SchemaVersion1,
    pub scenario_id: String,
    pub model: ModelFixtureV1,
    /// How `router::chat` selects an unused generation. Existing fixtures use
    /// strict ordinal dispatch; concurrent tree fixtures opt into matching the
    /// first unused generation whose request contract passes.
    #[serde(default)]
    pub dispatch: RouterDispatchV1,
    pub generations: Vec<ScriptedGenerationV1>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RouterDispatchV1 {
    #[default]
    Ordinal,
    MatchAny,
}

/// The literal string `"1"`; any other value is a schema error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SchemaVersion1 {
    #[serde(rename = "1")]
    V1,
}

/// Mirror of the catalog `Model` (`llm-router/src/types/model.rs:43`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelFixtureV1 {
    pub id: String,
    pub provider: String,
    pub context_window: u64,
    pub max_output_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_thinking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_xhigh: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_tools: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_cache: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_structured_output: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum JsonMatcherV1 {
    Absent,
    Regex {
        pattern: String,
    },
    Sha256 {
        expected: String,
    },
    Exact {
        expected: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        normalize: Option<Vec<JsonNormalizerV1>>,
    },
    Subset {
        expected: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        normalize: Option<Vec<JsonNormalizerV1>>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JsonNormalizerV1 {
    /// RFC 6901 JSON Pointer.
    pub pointer: String,
    pub operation: NormalizerOperation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NormalizerOperation {
    Delete,
}

/// The 12 router-request fields; compiled scenarios always make every
/// matcher explicit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GenerationMatchV1 {
    pub writer_ref: JsonMatcherV1,
    pub request_id: JsonMatcherV1,
    pub model: JsonMatcherV1,
    pub provider: JsonMatcherV1,
    pub system_prompt: JsonMatcherV1,
    pub messages: JsonMatcherV1,
    pub tools: JsonMatcherV1,
    pub response_format: JsonMatcherV1,
    pub thinking_level: JsonMatcherV1,
    pub max_output_tokens: JsonMatcherV1,
    pub provider_options: JsonMatcherV1,
    pub metadata: JsonMatcherV1,
}

impl GenerationMatchV1 {
    pub fn uniform(matcher: JsonMatcherV1) -> Self {
        Self {
            writer_ref: matcher.clone(),
            request_id: matcher.clone(),
            model: matcher.clone(),
            provider: matcher.clone(),
            system_prompt: matcher.clone(),
            messages: matcher.clone(),
            tools: matcher.clone(),
            response_format: matcher.clone(),
            thinking_level: matcher.clone(),
            max_output_tokens: matcher.clone(),
            provider_options: matcher.clone(),
            metadata: matcher,
        }
    }

    /// Field name → matcher, in the canonical field order.
    pub fn fields(&self) -> [(&'static str, &JsonMatcherV1); 12] {
        [
            ("writer_ref", &self.writer_ref),
            ("request_id", &self.request_id),
            ("model", &self.model),
            ("provider", &self.provider),
            ("system_prompt", &self.system_prompt),
            ("messages", &self.messages),
            ("tools", &self.tools),
            ("response_format", &self.response_format),
            ("thinking_level", &self.thinking_level),
            ("max_output_tokens", &self.max_output_tokens),
            ("provider_options", &self.provider_options),
            ("metadata", &self.metadata),
        ]
    }
}

/// A serve-time capture: after its generation MATCHES, `pattern` runs over
/// the raw serialized request and the first capture group is stored under
/// `name`. Later (or same-ordinal) generations' frames may reference it as
/// `[[cap:<name>]]` — the only way a fixture can echo a RUNTIME-generated
/// identifier (e.g. a `sub_…` subscription id) back into a scripted call.
/// Distinct from `{{…}}` placeholders, which expand before the stack boots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureV1 {
    pub name: String,
    /// Regex with exactly one capture group.
    pub pattern: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScriptedGenerationV1 {
    pub ordinal: u64,
    #[serde(rename = "match")]
    pub match_: GenerationMatchV1,
    /// Runtime-value captures taken from this generation's matched request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captures: Vec<CaptureV1>,
    pub frames: Vec<AssistantMessageEvent>,
    pub response: RouterChatResponse,
    /// Hold the matched router call at a named runner-controlled barrier
    /// before any response frame is sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<ServeGateV1>,
    /// Optional side effect the router performs *before* it streams this
    /// generation's frames (or fails the call): an awaited steer send. Because
    /// the harness is blocked awaiting this generation (its turn is `Running`),
    /// the steered message parks durably in the turn's queue before the
    /// generation's outcome proceeds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_serve: Option<ServeEffectV1>,
    /// When set, the router streams nothing: after `on_serve`, the call fails
    /// with this handler-error message. The harness classifies a router
    /// handler error as permanent and finalizes the turn `failed` WITHOUT the
    /// completed path's steering check — the only deterministic public-path
    /// route into the finalize drain with a parked message present, which the
    /// reseed regression scenario requires. `frames` must be empty and
    /// `response` is never sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

/// A serve-time side effect. Kept as a struct (not a bare field) so future
/// effects can be added without another optional column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServeEffectV1 {
    pub steer: SteerSendV1,
}

/// An awaited `harness::send` into a live session. The router issues it before
/// streaming, so the message is durably enqueued before the frames (and thus
/// the turn's finalize) proceed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SteerSendV1 {
    /// Target session, placeholder-expanded (e.g. `"{{session_id}}"`).
    pub session_id: String,
    /// User-message text steered into the session before streaming.
    pub message: String,
}

/// A named barrier shared by the runner and one or more scripted calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServeGateV1 {
    pub name: String,
}
