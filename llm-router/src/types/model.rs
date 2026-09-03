use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// "minimal" requests the lowest reasoning effort and needs only `thinking`
/// support; levels map to provider-native knobs via `Model::thinking_budgets`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

/// One provider-native reasoning effort advertised for a specific model.
///
/// Values intentionally remain strings: provider catalogs can add efforts
/// without requiring a router-wide enum release first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReasoningEffort {
    pub effort: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pricing {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
}

/// What a speech model does. Chat models carry no `speech` block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SpeechModality {
    /// Speech to text, served through `router::transcribe`.
    Stt,
    /// Text to speech, served through `router::speak`.
    Tts,
}

/// Facts about a speech model. Present only on models served through
/// `router::transcribe` / `router::speak`; such models report
/// `context_window` and `max_output_tokens` as 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SpeechModel {
    pub modality: SpeechModality,
    /// BCP-47 tags of the languages the model handles; empty when the
    /// provider does not say.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    /// Realtime input (stt) or streamed audio output (tts) is available.
    #[serde(default)]
    pub streaming: bool,
}

/// Model family selector for `router::models::list`. `chat` is the default
/// so pickers built before speech models existed keep listing only what
/// `router::chat` can run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ModalityFilter {
    #[default]
    Chat,
    Stt,
    Tts,
    Any,
}

/// The capability record (README § Model descriptor).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Model {
    pub id: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub context_window: u64,
    pub max_output_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_thinking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_xhigh: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_efforts: Option<Vec<ReasoningEffort>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_tools: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_cache: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_structured_output: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_budgets: Option<BTreeMap<ThinkingLevel, u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<Pricing>,
    /// Set on speech models only; see [`SpeechModel`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech: Option<SpeechModel>,
}

impl Model {
    /// `None` for chat models.
    pub fn speech_modality(&self) -> Option<SpeechModality> {
        self.speech.as_ref().map(|s| s.modality)
    }

    pub fn matches_modality(&self, filter: ModalityFilter) -> bool {
        match filter {
            ModalityFilter::Any => true,
            ModalityFilter::Chat => self.speech.is_none(),
            ModalityFilter::Stt => self.speech_modality() == Some(SpeechModality::Stt),
            ModalityFilter::Tts => self.speech_modality() == Some(SpeechModality::Tts),
        }
    }
}

/// Function invocation schema — what a provider sees as a `tools` array entry
/// (README § Function invocation schema; adapter boundary). These describe iii
/// functions exposed to the model, not provider-native tools.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value, // JSON Schema of the arguments
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>, // "parallel" | "sequential"
}
