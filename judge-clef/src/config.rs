//! Operator-owned model choice, hardware placement and execution limits. No
//! credentials: the model runs in-process from public Hugging Face files.
use crate::{download::MODELS, Limits};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ClefConfig {
    /// Pinned Clef checkpoint (one GGUF: backbone and joint schema head).
    /// Applied at the next worker start.
    pub model: String,
    /// CPU threads for llama.cpp's CPU work (all of it with `gpu_layers: 0`),
    /// applied at the next start. Hybrid CPUs run faster around their
    /// performance-core count.
    #[schemars(range(min = 1, max = 256))]
    pub threads: usize,
    /// Layers offloaded to the GPU (Vulkan or Metal builds), applied at the
    /// next start. Null offloads the whole model, joint schema head included,
    /// when a GPU is present; 0 runs it all on the CPU.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_layers: Option<u32>,
    /// Longest prompt (state and every question with its options) in tokens,
    /// applied at the next start. A longer state is truncated, keeping its
    /// beginning; a schema longer than the window answers `payload_too_large`.
    /// Each evaluation is one pass whose GPU buffers grow with its prompt:
    /// 10.1 GiB of VRAM at 16384, the most that ran safely on a 16 GiB
    /// GPU (24576 lost the device).
    #[schemars(range(min = 512, max = 16384))]
    pub context_tokens: u32,
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

impl Default for ClefConfig {
    fn default() -> Self {
        let limits = Limits::default();
        Self {
            model: MODELS[0].name.into(),
            threads: default_threads(),
            gpu_layers: None,
            // The reference's max_length.
            context_tokens: 16384,
            max_request_bytes: limits.max_request_bytes,
            max_timeout_ms: limits.max_timeout_ms,
        }
    }
}
impl ClefConfig {
    pub fn validate(&self) -> Result<(), String> {
        if crate::download::model(&self.model).is_none() {
            let names: Vec<_> = MODELS.iter().map(|m| m.name).collect();
            return Err(format!("clef model must be one of {names:?}"));
        }
        if !(1..=256).contains(&self.threads)
            || !(512..=16_384).contains(&self.context_tokens)
            || self.max_request_bytes == 0
            || self.max_timeout_ms == 0
            || i64::try_from(self.max_timeout_ms).is_err()
        {
            return Err("Invalid clef execution limits".into());
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
            .map_err(|_| "Invalid clef configuration".to_string())?;
        config.validate()?;
        Ok(config)
    }
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("clef config serializes")
    }
    pub fn json_schema() -> Value {
        let mut schema =
            serde_json::to_value(schemars::schema_for!(Self)).expect("clef schema serializes");
        schema["properties"]["model"]["enum"] =
            serde_json::json!(MODELS.iter().map(|m| m.name).collect::<Vec<_>>());
        schema
    }
}
