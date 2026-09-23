//! Operator-owned model choice, hardware placement and execution limits. No
//! credentials: the model runs in-process from a public GGUF.
use crate::{download::MODELS, Limits};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SemifConfig {
    /// Pinned GGUF checkpoint. Applied at the next worker start.
    pub model: String,
    /// CPU threads for prompt processing, applied at the next start. Hybrid
    /// CPUs run faster around their performance-core count.
    #[schemars(range(min = 1, max = 256))]
    pub threads: usize,
    /// Transformer layers offloaded to the GPU (Vulkan or Metal builds), applied
    /// at the next start. Null offloads every layer when a GPU is present; 0
    /// keeps the model on the CPU.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_layers: Option<u32>,
    /// Context window in tokens, applied at the next start. A prompt (state,
    /// question and options) longer than this answers `payload_too_large`:
    /// SemIf never truncates evidence.
    #[schemars(range(min = 512, max = 262144))]
    pub context_tokens: u32,
    /// Questions about one state decoded together in a batch, applied at the
    /// next start. Pays off on GPUs; 1 decodes them one at a time. Their
    /// prompts share the context window, so long states run fewer at once.
    #[schemars(range(min = 1, max = 64))]
    pub parallel_questions: usize,
    /// Maximum encoded JSON bytes per evaluation.
    #[schemars(range(min = 1))]
    pub max_request_bytes: usize,
    /// Maximum relative deadline for evaluations and model listing.
    #[schemars(range(min = 1))]
    pub max_timeout_ms: u64,
}

/// Hybrid desktop CPUs slow down past their performance cores; cap the default.
pub fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8)
}

impl Default for SemifConfig {
    fn default() -> Self {
        let limits = Limits::default();
        Self {
            model: MODELS[0].name.into(),
            threads: default_threads(),
            gpu_layers: None,
            context_tokens: 16384,
            parallel_questions: 8,
            max_request_bytes: limits.max_request_bytes,
            max_timeout_ms: limits.max_timeout_ms,
        }
    }
}
impl SemifConfig {
    pub fn validate(&self) -> Result<(), String> {
        if crate::download::model(&self.model).is_none() {
            let names: Vec<_> = MODELS.iter().map(|m| m.name).collect();
            return Err(format!("SemIf model must be one of {names:?}"));
        }
        if !(1..=256).contains(&self.threads)
            || !(512..=262_144).contains(&self.context_tokens)
            || !(1..=64).contains(&self.parallel_questions)
            || self.max_request_bytes == 0
            || self.max_timeout_ms == 0
            || i64::try_from(self.max_timeout_ms).is_err()
        {
            return Err("Invalid SemIf execution limits".into());
        }
        Ok(())
    }
    pub fn limits(&self) -> Limits {
        Limits {
            max_request_bytes: self.max_request_bytes,
            max_timeout_ms: self.max_timeout_ms,
        }
    }
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let config: Self = serde_json::from_value(value.clone())
            .map_err(|_| "Invalid SemIf configuration".to_string())?;
        config.validate()?;
        Ok(config)
    }
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("SemIf config serializes")
    }
    pub fn json_schema() -> Value {
        let mut schema =
            serde_json::to_value(schemars::schema_for!(Self)).expect("SemIf schema serializes");
        schema["properties"]["model"]["enum"] =
            serde_json::json!(MODELS.iter().map(|m| m.name).collect::<Vec<_>>());
        schema
    }
}
