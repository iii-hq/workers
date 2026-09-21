//! Typed JEV messages; no SDK, credentials, transport or retrieval policy.
mod answers;
mod encoding;
mod options;
mod questions;
pub use options::{RequestOptions, RetryPolicy};

pub use answers::{validate_answer, Answer};
pub use encoding::{
    encode_evaluation, encode_evaluation_with_limits, validate_request,
    validate_request_with_limits,
};
pub use questions::{Content, Question, ScoreLevel};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const FUNCTION_ID: &str = "jev::evaluate";
pub const MODELS_FUNCTION_ID: &str = "jev::models::list";
pub const CANCEL_FUNCTION_ID: &str = "jev::cancel";
pub const DEFAULT_MODEL: &str = "jev-1.13.0";
pub const MAX_EVALUATIONS: usize = 512;
pub const DEFAULT_MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
pub const DEFAULT_MAX_TIMEOUT_MS: u64 = 300_000;

#[derive(Clone, Copy, Debug)]
pub struct EncodingLimits {
    pub max_body_bytes: usize,
    pub max_state_question_bytes: Option<usize>,
}
impl Default for EncodingLimits {
    fn default() -> Self {
        Self {
            max_body_bytes: DEFAULT_MAX_REQUEST_BYTES,
            max_state_question_bytes: None,
        }
    }
}
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
