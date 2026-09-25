//! Typed judge messages shared by the `judge` hub and every `judge-<provider>`
//! worker; no SDK, credentials, transport or retrieval policy.
mod answers;
pub mod confidence;
mod encoding;
mod options;
mod questions;
pub use options::RequestOptions;

pub use answers::{unique_map, validate_answer, Answer};
pub use encoding::{
    encode_evaluation, encode_evaluation_with_limits, validate_request,
    validate_request_with_limits,
};
pub use questions::{Content, Question, ScoreLevel};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const FUNCTION_ID: &str = "judge::evaluate";
pub const MODELS_FUNCTION_ID: &str = "judge::models::list";
pub const CANCEL_FUNCTION_ID: &str = "judge::cancel";
/// The hub forwards each public function to `judge-<provider>::<surface>`;
/// provider workers register exactly these ids, as internal functions.
pub fn provider_function_id(provider: &str, public_id: &str) -> String {
    format!(
        "judge-{provider}::{}",
        public_id.trim_start_matches("judge::")
    )
}
/// OTel baggage key carrying the calling session's judge provider, stamped by
/// the harness on every turn. Callers inside that turn send it as the
/// request's `provider`; the hub falls back to it when a request names none.
/// Caller-supplied and unauthenticated: a routing preference, never an
/// access decision.
pub const PROVIDER_BAGGAGE_KEY: &str = "iii.judge.provider";

/// `judge-<provider>` suffixes are worker names: lowercase letters, digits and
/// hyphens, at most 64 bytes.
pub fn is_valid_provider(provider: &str) -> bool {
    !provider.is_empty()
        && provider.len() <= 64
        && provider
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub const MAX_EVALUATIONS: usize = 512;
/// Provider-side `request_id` bound. Public ids are at most 128 bytes; the hub
/// forwards them as `<caller length>:<caller>/<request_id>`, and rejects the
/// composed id when it would exceed this.
pub const MAX_PROVIDER_REQUEST_ID_BYTES: usize = 512;
pub const DEFAULT_MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
pub const DEFAULT_MAX_TIMEOUT_MS: u64 = 300_000;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvaluateRequest {
    #[serde(default)]
    pub options: RequestOptions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 128))]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[schemars(range(min = 1))]
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_unix_ms: Option<u64>,
    #[schemars(length(min = 1, max = 512))]
    pub evaluations: Vec<Evaluation>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evaluation {
    pub id: String,
    #[schemars(with = "ScoreLevel")]
    pub state: Value,
    #[serde(deserialize_with = "answers::unique_map")]
    #[schemars(schema_with = "questions::questions_schema")]
    pub questions: BTreeMap<String, Question>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvaluationResult {
    #[serde(deserialize_with = "answers::unique_map")]
    pub answers: BTreeMap<String, Answer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}
/// Each absent/null counter is unknown, independently of the other counter.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}
/// Bounded provider diagnostics, with credentials and supplied header values redacted.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderError {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default)]
    pub truncated: bool,
}
/// Known accepted usage, never a billing estimate. Unreported counters or
/// failure/cancellation make usage_complete false.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stats {
    pub attempts: usize,
    pub requests: usize,
    pub questions: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub elapsed_ms: u64,
    pub usage_complete: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum EvaluateResponse {
    Ok {
        model: String,
        results: BTreeMap<String, EvaluationResult>,
        stats: Stats,
    },
    Error {
        code: ErrorCode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        http_status: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_error: Option<ProviderError>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_ms: Option<u64>,
        stats: Stats,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ModelsRequest {
    pub options: RequestOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 128))]
    pub request_id: Option<String>,
    #[schemars(range(min = 1))]
    pub timeout_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at_unix_ms: Option<u64>,
}
impl Default for ModelsRequest {
    fn default() -> Self {
        Self {
            options: RequestOptions::default(),
            request_id: None,
            timeout_ms: 30_000,
            expires_at_unix_ms: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ModelCard {
    pub name: String,
    pub description: String,
    pub release_date: String,
    /// Tokens one evaluation row can hold (question, options and state), when
    /// the provider has a fixed window. Callers size their states from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelsResponse {
    Ok {
        models: Vec<ModelCard>,
        stats: Stats,
    },
    Error {
        code: ErrorCode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        http_status: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_error: Option<ProviderError>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_ms: Option<u64>,
        stats: Stats,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    MissingKey,
    PayloadTooLarge,
    Deadline,
    AttemptTimeout,
    Cancelled,
    Http,
    Transport,
    InvalidResponse,
    /// The selected `judge-<provider>` worker is not registered on the engine.
    ProviderUnavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancelRequest {
    #[schemars(length(min = 1, max = 128))]
    pub request_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum CancelResponse {
    Ok { cancelled: bool },
    Error { code: ErrorCode },
}
