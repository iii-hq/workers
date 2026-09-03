//! Operator-facing runtime configuration.
//!
//! The authoritative value comes from the `configuration` worker at boot
//! (see [`crate::configuration`]); a `--config` YAML file, when passed, only
//! SEEDS the initial registration. Every field has a serde default so an empty
//! object yields a fully-populated config.
//!
//! Speech-to-text runs locally by default: a small streaming model is
//! downloaded on first use into `models_dir` and nothing leaves the machine.
//! Pointing `stt.backend` at an OpenAI-compatible audio endpoint (a local
//! whisper server or a hosted API) swaps the engine without touching callers.
//! Read-aloud has no local engine in this release: it uses the host's own
//! speech command or an OpenAI-compatible speech endpoint.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Root config shape. Unknown keys are rejected so a typo'd field fails loudly
/// instead of silently running the default.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Where downloaded speech models live. Relative paths resolve against the
    /// Compose project directory (or the process directory when standalone).
    #[serde(default = "default_models_dir")]
    pub models_dir: String,

    /// Speech-to-text settings: the local model and its endpointing, or an
    /// OpenAI-compatible transcription endpoint.
    #[serde(default)]
    pub stt: SttConfig,

    /// Read-aloud settings.
    #[serde(default)]
    pub tts: TtsConfig,

    /// Largest inline audio payload accepted by `voice::transcribe`, in bytes
    /// (decoded). Larger files must be passed by path.
    #[serde(default = "default_max_audio_bytes")]
    pub max_audio_bytes: usize,

    /// Most dictation sessions kept open at once across every caller.
    #[serde(default = "default_max_sessions")]
    pub max_sessions: usize,

    /// Seconds a dictation session may sit without audio before the worker
    /// closes it and emits its final transcript.
    #[serde(default = "default_session_idle_secs")]
    pub session_idle_secs: u64,
}

/// Which speech-to-text engine answers.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SttBackend {
    /// The bundled streaming recognizer with a model from `models_dir`.
    Local,
    /// An OpenAI-compatible `/v1/audio/transcriptions` endpoint.
    Openai,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SttConfig {
    /// `local` (default) or `openai`.
    #[serde(default = "default_stt_backend")]
    pub backend: SttBackend,

    /// Local streaming model id from `voice::models::list`, the one that
    /// produces live partial text. Downloaded on first use.
    #[serde(default = "default_model")]
    pub model: String,

    /// Second-pass model id that re-decodes each finished utterance for the
    /// final text (punctuation, casing, far better accuracy than the
    /// streaming model). Downloaded in the background on first use; until it
    /// is present the streaming text stands. Empty disables the second pass.
    #[serde(default = "default_final_model")]
    pub final_model: String,

    /// Decoder threads for the local model.
    #[serde(default = "default_num_threads")]
    pub num_threads: usize,

    /// Trailing silence, in seconds, that ends an utterance once speech has
    /// been heard. Lower commits dictated text sooner; higher tolerates pauses.
    #[serde(default = "default_silence_after_speech_secs")]
    pub silence_after_speech_secs: f32,

    /// Trailing silence, in seconds, that ends a segment when nothing has been
    /// recognized yet. Keep this well above the model's lookahead so the
    /// first words of an utterance are never cut.
    #[serde(default = "default_silence_without_speech_secs")]
    pub silence_without_speech_secs: f32,

    /// Longest single utterance, in seconds, before the recognizer commits
    /// what it has and starts a new segment.
    #[serde(default = "default_max_utterance_secs")]
    pub max_utterance_secs: f32,

    /// Settings for the `openai` backend.
    #[serde(default)]
    pub openai: OpenaiSttConfig,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenaiSttConfig {
    /// Base URL of the OpenAI-compatible API, e.g. `https://api.openai.com/v1`
    /// or a local whisper server.
    #[serde(default = "default_openai_base_url")]
    pub base_url: String,

    /// Bearer token. Use `${OPENAI_API_KEY}` to read it from the engine's
    /// environment; leave empty for servers that need none.
    #[serde(default)]
    pub api_key: String,

    /// Transcription model name the endpoint expects.
    #[serde(default = "default_openai_stt_model")]
    pub model: String,

    /// Language hint (ISO 639-1) sent with each request. Empty means detect.
    #[serde(default)]
    pub language: String,
}

/// Which read-aloud engine answers.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TtsBackend {
    /// The host's own speech command: `say` on macOS, `espeak-ng` (or
    /// `espeak`) on Linux. Audio plays on the machine running the worker.
    Host,
    /// An OpenAI-compatible `/v1/audio/speech` endpoint. Audio is returned to
    /// the caller for playback in the browser.
    Openai,
    /// Read-aloud disabled.
    Off,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TtsConfig {
    /// `host` (default), `openai`, or `off`.
    #[serde(default = "default_tts_backend")]
    pub backend: TtsBackend,

    /// Voice name passed to the host speech command (`say -v`, `espeak-ng -v`).
    /// Empty uses the system default.
    #[serde(default)]
    pub voice: String,

    /// Speaking rate for the host speech command, in words per minute. 0 uses
    /// the command's default.
    #[serde(default)]
    pub rate_wpm: u32,

    /// Longest text, in characters, one `voice::speak` call will read.
    #[serde(default = "default_max_speak_chars")]
    pub max_speak_chars: usize,

    /// Settings for the `openai` backend.
    #[serde(default)]
    pub openai: OpenaiTtsConfig,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenaiTtsConfig {
    /// Base URL of the OpenAI-compatible API.
    #[serde(default = "default_openai_base_url")]
    pub base_url: String,

    /// Bearer token. Use `${OPENAI_API_KEY}` to read it from the engine's
    /// environment; leave empty for servers that need none.
    #[serde(default)]
    pub api_key: String,

    /// Speech model name the endpoint expects.
    #[serde(default = "default_openai_tts_model")]
    pub model: String,

    /// Voice name the endpoint expects.
    #[serde(default = "default_openai_tts_voice")]
    pub voice: String,
}

fn default_models_dir() -> String {
    iii_worker_paths::default_path("data/voice/models")
}

fn default_max_audio_bytes() -> usize {
    10 * 1024 * 1024
}

fn default_max_sessions() -> usize {
    8
}

fn default_session_idle_secs() -> u64 {
    120
}

fn default_stt_backend() -> SttBackend {
    SttBackend::Local
}

fn default_model() -> String {
    crate::models::DEFAULT_MODEL.to_string()
}

fn default_final_model() -> String {
    crate::models::DEFAULT_FINAL_MODEL.to_string()
}

fn default_num_threads() -> usize {
    2
}

fn default_silence_after_speech_secs() -> f32 {
    0.8
}

fn default_silence_without_speech_secs() -> f32 {
    2.4
}

fn default_max_utterance_secs() -> f32 {
    20.0
}

fn default_openai_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_openai_stt_model() -> String {
    "whisper-1".to_string()
}

fn default_tts_backend() -> TtsBackend {
    TtsBackend::Host
}

fn default_max_speak_chars() -> usize {
    4000
}

fn default_openai_tts_model() -> String {
    "tts-1".to_string()
}

fn default_openai_tts_voice() -> String {
    "alloy".to_string()
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            models_dir: default_models_dir(),
            stt: SttConfig::default(),
            tts: TtsConfig::default(),
            max_audio_bytes: default_max_audio_bytes(),
            max_sessions: default_max_sessions(),
            session_idle_secs: default_session_idle_secs(),
        }
    }
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            backend: default_stt_backend(),
            model: default_model(),
            final_model: default_final_model(),
            num_threads: default_num_threads(),
            silence_after_speech_secs: default_silence_after_speech_secs(),
            silence_without_speech_secs: default_silence_without_speech_secs(),
            max_utterance_secs: default_max_utterance_secs(),
            openai: OpenaiSttConfig::default(),
        }
    }
}

impl Default for OpenaiSttConfig {
    fn default() -> Self {
        Self {
            base_url: default_openai_base_url(),
            api_key: String::new(),
            model: default_openai_stt_model(),
            language: String::new(),
        }
    }
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            backend: default_tts_backend(),
            voice: String::new(),
            rate_wpm: 0,
            max_speak_chars: default_max_speak_chars(),
            openai: OpenaiTtsConfig::default(),
        }
    }
}

impl Default for OpenaiTtsConfig {
    fn default() -> Self {
        Self {
            base_url: default_openai_base_url(),
            api_key: String::new(),
            model: default_openai_tts_model(),
            voice: default_openai_tts_voice(),
        }
    }
}

impl WorkerConfig {
    /// Parse a seed config from YAML, expanding `${NAME}` against the process
    /// env FIRST (the seed file is the only path that needs expansion — values
    /// fetched from `configuration::get` are already env-expanded by the
    /// configuration worker), then deserializing.
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let expanded = expand_env(yaml);
        let parsed: Self =
            serde_yaml::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))?;
        parsed.validate()
    }

    /// Reject values that parse but cannot mean anything.
    fn validate(self) -> Result<Self, String> {
        if self.models_dir.trim().is_empty() {
            return Err("models_dir must not be empty".to_string());
        }
        if self.max_audio_bytes == 0 {
            return Err("max_audio_bytes must be at least 1".to_string());
        }
        if self.max_sessions == 0 {
            return Err("max_sessions must be at least 1".to_string());
        }
        if self.stt.model.trim().is_empty() {
            return Err("stt.model must not be empty".to_string());
        }
        if self.stt.num_threads == 0 {
            return Err("stt.num_threads must be at least 1".to_string());
        }
        for (name, value) in [
            (
                "stt.silence_after_speech_secs",
                self.stt.silence_after_speech_secs,
            ),
            (
                "stt.silence_without_speech_secs",
                self.stt.silence_without_speech_secs,
            ),
            ("stt.max_utterance_secs", self.stt.max_utterance_secs),
        ] {
            if !(value.is_finite() && value > 0.0) {
                return Err(format!("{name} must be a positive number"));
            }
        }
        if self.tts.max_speak_chars == 0 {
            return Err("tts.max_speak_chars must be at least 1".to_string());
        }
        Ok(self)
    }

    /// Read and parse a YAML seed file (env-expanded — see [`Self::from_yaml`]).
    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    /// Parse a config from a JSON value already env-expanded by the
    /// configuration worker. Does NOT run [`expand_env`] (double expansion
    /// would be a bug) and tolerates a zero-field object (serde defaults fill
    /// in).
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let parsed: Self =
            serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))?;
        parsed.validate()
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    /// The JSON Schema registered with the `configuration` worker. Field
    /// doc-comments become property descriptions; the shipped defaults are
    /// attached as a top-level `example`.
    pub fn json_schema() -> Value {
        let root = schemars::schema_for!(WorkerConfig);
        let mut schema =
            serde_json::to_value(&root.schema).expect("WorkerConfig JSON Schema serializes");
        if let Some(obj) = schema.as_object_mut() {
            if !root.definitions.is_empty() {
                obj.insert(
                    "definitions".into(),
                    serde_json::to_value(&root.definitions).expect("definitions serialize"),
                );
            }
            obj.insert("example".into(), WorkerConfig::default().to_json());
        }
        schema
    }

    /// The models directory as a filesystem path.
    pub fn models_path(&self) -> std::path::PathBuf {
        iii_worker_paths::resolve_path(&self.models_dir)
    }
}

/// Expand `${NAME}` and `${NAME:default}` against the process env. An unset
/// variable with no default expands to the empty string, matching the
/// configuration worker's own expansion.
fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let spec = &after[..end];
                let (name, fallback) = match spec.split_once(':') {
                    Some((n, d)) => (n, Some(d)),
                    None => (spec, None),
                };
                match (std::env::var(name), fallback) {
                    (Ok(v), _) => out.push_str(&v),
                    (Err(_), Some(d)) => out.push_str(d),
                    (Err(_), None) => {
                        tracing::warn!(var = %name, "config references undefined env var")
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_object_is_the_full_default() {
        let parsed = WorkerConfig::from_json(&serde_json::json!({})).expect("parses");
        assert_eq!(parsed, WorkerConfig::default());
        assert_eq!(parsed.stt.backend, SttBackend::Local);
        assert_eq!(parsed.tts.backend, TtsBackend::Host);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let err = WorkerConfig::from_json(&serde_json::json!({ "modle": "x" })).unwrap_err();
        assert!(err.contains("unknown field"), "{err}");
    }

    #[test]
    fn nonsense_values_are_rejected() {
        let err = WorkerConfig::from_json(&serde_json::json!({ "max_sessions": 0 })).unwrap_err();
        assert!(err.contains("max_sessions"), "{err}");
        let err = WorkerConfig::from_json(
            &serde_json::json!({ "stt": { "silence_after_speech_secs": -1.0 } }),
        )
        .unwrap_err();
        assert!(err.contains("silence_after_speech_secs"), "{err}");
    }

    #[test]
    fn yaml_seed_expands_env() {
        std::env::set_var("VOICE_TEST_KEY", "secret");
        let cfg = WorkerConfig::from_yaml(
            "stt:\n  backend: openai\n  openai:\n    api_key: ${VOICE_TEST_KEY}\n",
        )
        .expect("parses");
        assert_eq!(cfg.stt.backend, SttBackend::Openai);
        assert_eq!(cfg.stt.openai.api_key, "secret");
    }

    #[test]
    fn schema_carries_the_defaults_as_example() {
        let schema = WorkerConfig::json_schema();
        assert_eq!(schema["example"], WorkerConfig::default().to_json());
        assert!(schema["properties"]["stt"].is_object());
    }
}
