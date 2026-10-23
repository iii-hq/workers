//! Operator-owned checkpoint choice and execution limits. No credentials: the
//! model runs in-process from public Hugging Face files.
use crate::{download::MODELS, Limits, Routing};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LayaConfig {
    /// Default checkpoint: `laya` (English, ModernBERT-large), `laya-multilingual`
    /// (mmBERT-base, 100+ languages) or `laya-typed-decisions` (fine-tuned, 1024
    /// tokens). Changing it takes effect at the next worker start.
    pub model: String,
    /// Extra checkpoints loaded at the next start (about 1 GB of RAM each),
    /// selectable per request by `model` and used by routing.
    pub preload: Vec<String>,
    /// Route each evaluation without an explicit `model` by its state's script
    /// and language, like laya's Router: non-English states go to
    /// `laya-multilingual`, English ones to `laya`, when those are loaded;
    /// Latin text no word list identifies stays on the default `model`.
    pub auto_route: bool,
    /// Send an evaluation whose question ids form one of laya's four
    /// typed-decisions workflows to `laya-typed-decisions` when it is loaded.
    pub auto_task_detection: bool,
    /// Answer a choice of more than 16 options as laya's tournament (groups of
    /// 16, then a final over their winners) instead of one row whose labels
    /// get cut: more accurate on wide label sets, at one more forward pass,
    /// and `confidence` covers the finalists only.
    pub choice_tournament: bool,
    /// Hugging Face revision of `convaiinnovations/laya`; null follows `main`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// CPU threads for the forward pass, applied at the next worker start.
    /// Hybrid CPUs run faster below their logical core count.
    #[schemars(range(min = 1, max = 256))]
    pub threads: usize,
    /// Layers offloaded to the GPU (Vulkan or Metal builds), applied at the next
    /// start. Null offloads every layer when a GPU is present; 0 keeps the
    /// model on the CPU.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_layers: Option<u32>,
    /// Questions per forward pass, at most 8192 tokens' worth (16 at a
    /// 512-token window, 8 at 1024); cancellation and deadlines are checked
    /// between batches.
    #[schemars(range(min = 1, max = 256))]
    pub batch_questions: usize,
    /// Maximum encoded request bytes.
    #[schemars(range(min = 1))]
    pub max_request_bytes: usize,
    /// Maximum relative deadline for evaluations and model listing.
    #[schemars(range(min = 1))]
    pub max_timeout_ms: u64,
}
/// Measured on an i9-14900K: 6–8 threads answer a question in ~0.3 s, all 32
/// logical cores in ~0.8 s. Cap the default; operators can raise it.
pub fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8)
}

impl Default for LayaConfig {
    fn default() -> Self {
        let limits = Limits::default();
        Self {
            model: "laya".into(),
            preload: Vec::new(),
            auto_route: false,
            auto_task_detection: false,
            choice_tournament: false,
            revision: None,
            threads: default_threads(),
            gpu_layers: None,
            batch_questions: limits.batch_questions,
            max_request_bytes: limits.max_request_bytes,
            max_timeout_ms: limits.max_timeout_ms,
        }
    }
}
impl LayaConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !MODELS.contains(&self.model.as_str()) {
            return Err(format!("laya model must be one of {MODELS:?}"));
        }
        for (i, extra) in self.preload.iter().enumerate() {
            if !MODELS.contains(&extra.as_str()) {
                return Err(format!("laya preload entries must be one of {MODELS:?}"));
            }
            if *extra == self.model || self.preload[..i].contains(extra) {
                return Err(
                    "laya preload entries must be distinct from model and each other".into(),
                );
            }
        }
        if self.revision.as_deref().is_some_and(|r| {
            r.trim().is_empty()
                || !r
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_')
        }) {
            return Err("laya revision must be a Hugging Face branch, tag or commit".into());
        }
        if self.threads == 0
            || self.threads > 256
            || self.batch_questions == 0
            || self.batch_questions > 256
            || self.max_request_bytes == 0
            || self.max_timeout_ms == 0
            || i64::try_from(self.max_timeout_ms).is_err()
        {
            return Err("Invalid laya execution limits".into());
        }
        Ok(())
    }
    pub fn limits(&self) -> Limits {
        Limits {
            max_request_bytes: self.max_request_bytes,
            max_timeout_ms: self.max_timeout_ms,
            batch_questions: self.batch_questions,
        }
    }
    pub fn routing(&self) -> Routing {
        Routing {
            auto_route: self.auto_route,
            auto_task_detection: self.auto_task_detection,
            choice_tournament: self.choice_tournament,
        }
    }
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let config: Self = serde_json::from_value(value.clone())
            .map_err(|_| "Invalid laya configuration".to_string())?;
        config.validate()?;
        Ok(config)
    }
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("laya config serializes")
    }
    pub fn json_schema() -> Value {
        let mut schema =
            serde_json::to_value(schemars::schema_for!(Self)).expect("laya schema serializes");
        schema["properties"]["model"]["enum"] = serde_json::json!(MODELS);
        schema["properties"]["preload"]["items"]["enum"] = serde_json::json!(MODELS);
        schema
    }
}
